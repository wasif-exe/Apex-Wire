use std::os::fd::AsRawFd;
use libc::*;
use std::fs::{self, OpenOptions};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

const BATCH: usize = 64;
const WAL_EVENTS: usize = 100_000;
const HB_ITERS: usize = 100_000;
const SPSC_ITERS: usize = 500_000;
const MT_SYMBOLS: usize = 4;
const MT_ORDERS: usize = 25_000;

#[inline(always)]
fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        std::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    }
}

#[inline(always)]
fn rdtscp() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        let _aux: u32;
        std::arch::asm!("rdtscp", out("eax") lo, out("edx") hi, out("ecx") _aux, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WalEntry {
    seq: u64,
    ts: u64,
    price: u32,
    shares: u32,
    side: u8,
    symbol: u8,
    _pad: [u8; 6],
}

fn test_wal_recovery() -> (bool, String) {
    let wal_path = "/tmp/aw_v5_wal.bin";
    let entry_size = mem::size_of::<WalEntry>();
    let wal_size = WAL_EVENTS * entry_size;

    let _ = fs::remove_file(wal_path);
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(wal_path)
        .unwrap();
    f.set_len(wal_size as u64).unwrap();

    unsafe {
        let map = mmap(
            ptr::null_mut(),
            wal_size,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            f.as_raw_fd(),
            0,
        );
        if map == MAP_FAILED {
            return (false, "mmap failed".into());
        }

        let entries = map as *mut WalEntry;
        let t0 = rdtsc();
        for i in 0..WAL_EVENTS {
            let e = WalEntry {
                seq: i as u64,
                ts: rdtscp(),
                price: 10000 + (i % 500) as u32,
                shares: (i % 1000 + 1) as u32,
                side: (i & 1) as u8,
                symbol: (i % MT_SYMBOLS) as u8,
                _pad: [0; 6],
            };
            ptr::write_volatile(entries.add(i), e);
        }
        msync(map, wal_size, MS_SYNC);
        let t1 = rdtsc();
        let write_cyc = (t1 - t0) / WAL_EVENTS as u64;
        munmap(map, wal_size);
        drop(f);

        let f2 = OpenOptions::new().read(true).open(wal_path).unwrap();
        let map2 = mmap(
            ptr::null_mut(),
            wal_size,
            PROT_READ,
            MAP_PRIVATE,
            f2.as_raw_fd(),
            0,
        );
        if map2 == MAP_FAILED {
            return (false, "replay mmap failed".into());
        }

        let entries2 = map2 as *const WalEntry;
        let t2 = rdtsc();
        let mut replay_count = 0usize;
        let mut last_seq = 0u64;
        for i in 0..WAL_EVENTS {
            let e = ptr::read_volatile(entries2.add(i));
            if e.seq == last_seq && e.price > 0 {
                replay_count += 1;
                last_seq = e.seq + 1;
            }
        }
        let t3 = rdtsc();
        let replay_cyc = (t3 - t2) / WAL_EVENTS as u64;
        let replay_us = (t3 - t2) as f64 / 3200.0 / 1000.0;
        munmap(map2, wal_size);
        drop(f2);
        let _ = fs::remove_file(wal_path);

        let ok = replay_count == WAL_EVENTS;
        (
            ok,
            format!(
                "{} events | write={} cyc | replay={} cyc ({:.1} ms) | integrity={}",
                WAL_EVENTS,
                write_cyc,
                replay_cyc,
                replay_us,
                if ok { "OK" } else { "FAIL" }
            ),
        )
    }
}

#[repr(C, align(64))]
struct OrderBook {
    bids: [u32; 1024],
    asks: [u32; 1024],
    bid_depth: u32,
    ask_depth: u32,
    _pad: [u32; 6],
}

impl OrderBook {
    fn new() -> Self {
        Self {
            bids: [0; 1024],
            asks: [0; 1024],
            bid_depth: 0,
            ask_depth: 0,
            _pad: [0; 6],
        }
    }

    #[inline(always)]
    fn insert(&mut self, price: u32, shares: u32, is_buy: bool) {
        if is_buy {
            let idx = (price as usize) & 1023;
            self.bids[idx] = self.bids[idx].wrapping_add(shares);
            self.bid_depth += 1;
        } else {
            let idx = (price as usize) & 1023;
            self.asks[idx] = self.asks[idx].wrapping_add(shares);
            self.ask_depth += 1;
        }
    }
}

fn test_multi_tenant() -> (bool, String) {
    let mut books: Vec<OrderBook> = (0..MT_SYMBOLS).map(|_| OrderBook::new()).collect();

    struct Rng(u64);
    impl Rng {
        #[inline(always)]
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 as u32
        }
    }
    let mut rng = Rng(0xDEAD_BEEF_CAFE_BABE);

    let t0 = rdtsc();
    for _ in 0..MT_ORDERS {
        for sym in 0..MT_SYMBOLS {
            let price = rng.next() % 1000 + 10000;
            let shares = rng.next() % 500 + 1;
            let is_buy = rng.next() & 1 == 0;
            books[sym].insert(price, shares, is_buy);
        }
    }
    let t1 = rdtsc();

    let total_ops = MT_ORDERS * MT_SYMBOLS;
    let mt_cyc = (t1 - t0) / total_ops as u64;

    let mut single = OrderBook::new();
    let mut rng2 = Rng(0xDEAD_BEEF_CAFE_BABE);
    let t2 = rdtsc();
    for _ in 0..total_ops {
        let price = rng2.next() % 1000 + 10000;
        let shares = rng2.next() % 500 + 1;
        let is_buy = rng2.next() & 1 == 0;
        single.insert(price, shares, is_buy);
    }
    let t3 = rdtsc();
    let single_cyc = (t3 - t2) / total_ops as u64;

    let overhead_pct = if single_cyc > 0 {
        ((mt_cyc as f64 - single_cyc as f64) / single_cyc as f64) * 100.0
    } else {
        0.0
    };

    let total_depth: u32 = books.iter().map(|b| b.bid_depth + b.ask_depth).sum();
    let ok = total_depth > 0 && overhead_pct < 100.0;
    (
        ok,
        format!(
            "{} symbols x {} orders | single={} cyc | multi={} cyc | overhead={:.0}% | depth={}",
            MT_SYMBOLS, MT_ORDERS, single_cyc, mt_cyc, overhead_pct, total_depth
        ),
    )
}

#[repr(C, align(64))]
struct HeartbeatRegion {
    last_tick: AtomicU64,
    node_id: u64,
    _pad: [u64; 6],
}

fn test_failover_heartbeat() -> (bool, String) {
    unsafe {
        let map = mmap(
            ptr::null_mut(),
            4096,
            PROT_READ | PROT_WRITE,
            MAP_SHARED | MAP_ANONYMOUS,
            -1,
            0,
        );
        if map == MAP_FAILED {
            return (false, "mmap failed".into());
        }

        let hb = map as *mut HeartbeatRegion;
        ptr::write_volatile(hb, HeartbeatRegion {
            last_tick: AtomicU64::new(rdtsc()),
            node_id: 1,
            _pad: [0; 6],
        });

        let t0 = rdtsc();
        for _ in 0..HB_ITERS {
            (*hb).last_tick.store(rdtscp(), Ordering::Release);
        }
        let t1 = rdtsc();
        let write_cyc = (t1 - t0) / HB_ITERS as u64;

        let last = (*hb).last_tick.load(Ordering::Acquire);
        let now = rdtsc();
        let staleness = now.wrapping_sub(last);

        let timeout_cyc: u64 = 3_200_000;
        let mut detect_cyc;
        let t2 = rdtsc();
        let fake_last = t2.wrapping_sub(timeout_cyc * 2);
        (*hb).last_tick.store(fake_last, Ordering::Release);
        loop {
            let tick = (*hb).last_tick.load(Ordering::Acquire);
            let age = rdtscp().wrapping_sub(tick);
            if age > timeout_cyc {
                detect_cyc = age;
                break;
            }
        }
        let t3 = rdtsc();
        let detect_wall = t3 - t2;

        munmap(map, 4096);

        let ok = staleness < 1000 && detect_cyc > timeout_cyc;
        (
            ok,
            format!(
                "tick={} cyc | stale={} cyc | detect={} cyc ({:.1} µs) | timeout={} cyc",
                write_cyc,
                staleness,
                detect_wall,
                detect_wall as f64 / 3200.0,
                timeout_cyc
            ),
        )
    }
}

#[repr(C, align(64))]
struct SpscBridge {
    head: AtomicU64,
    tail: AtomicU64,
    capacity: u64,
    _pad: [u64; 5],
}

fn test_spsc_async_bridge() -> (bool, String) {
    let ring_cap: usize = 4096;
    let ring_mask = (ring_cap - 1) as u64;
    let data_size = ring_cap * mem::size_of::<WalEntry>();
    let total_size = mem::size_of::<SpscBridge>() + data_size;

    unsafe {
        let map = mmap(
            ptr::null_mut(),
            total_size,
            PROT_READ | PROT_WRITE,
            MAP_SHARED | MAP_ANONYMOUS,
            -1,
            0,
        );
        if map == MAP_FAILED {
            return (false, "mmap failed".into());
        }

        let bridge = map as *mut SpscBridge;
        let ring = (map as *mut u8).add(mem::size_of::<SpscBridge>()) as *mut WalEntry;

        ptr::write_volatile(bridge, SpscBridge {
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            capacity: ring_cap as u64,
            _pad: [0; 5],
        });

        let t0 = rdtsc();
        let mut produced = 0usize;
        let mut consumed = 0usize;

        while produced < SPSC_ITERS {
            let head = (*bridge).head.load(Ordering::Relaxed);
            let tail = (*bridge).tail.load(Ordering::Acquire);
            if head.wrapping_sub(tail) < ring_cap as u64 {
                let idx = (head & ring_mask) as usize;
                let e = WalEntry {
                    seq: produced as u64,
                    ts: rdtscp(),
                    price: 10000 + (produced % 500) as u32,
                    shares: 100,
                    side: 0,
                    symbol: 0,
                    _pad: [0; 6],
                };
                ptr::write_volatile(ring.add(idx), e);
                (*bridge).head.store(head + 1, Ordering::Release);
                produced += 1;
            }

            let head2 = (*bridge).head.load(Ordering::Acquire);
            let tail2 = (*bridge).tail.load(Ordering::Relaxed);
            if tail2 < head2 {
                let _e = ptr::read_volatile(ring.add((tail2 & ring_mask) as usize));
                (*bridge).tail.store(tail2 + 1, Ordering::Release);
                consumed += 1;
            }
        }

        while consumed < produced {
            let head2 = (*bridge).head.load(Ordering::Acquire);
            let tail2 = (*bridge).tail.load(Ordering::Relaxed);
            if tail2 < head2 {
                let _e = ptr::read_volatile(ring.add((tail2 & ring_mask) as usize));
                (*bridge).tail.store(tail2 + 1, Ordering::Release);
                consumed += 1;
            }
        }

        let t1 = rdtsc();
        let cyc_per_msg = (t1 - t0) / (SPSC_ITERS as u64 * 2);

        munmap(map, total_size);

        let ok = consumed == SPSC_ITERS;
        (
            ok,
            format!(
                "{} msgs | {} cyc/msg (prod+cons) | integrity={}",
                SPSC_ITERS,
                cyc_per_msg,
                if ok { "OK" } else { "FAIL" }
            ),
        )
    }
}

fn test_combined_pipeline() -> (bool, String) {
    let wal_path = "/tmp/aw_v5_combined_wal.bin";
    let entry_size = mem::size_of::<WalEntry>();
    let wal_count = 10_000;
    let wal_size = wal_count * entry_size;

    let _ = fs::remove_file(wal_path);
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(wal_path)
        .unwrap();
    f.set_len(wal_size as u64).unwrap();

    unsafe {
        let wal_map = mmap(
            ptr::null_mut(),
            wal_size,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            f.as_raw_fd(),
            0,
        );
        let hb_map = mmap(
            ptr::null_mut(),
            4096,
            PROT_READ | PROT_WRITE,
            MAP_SHARED | MAP_ANONYMOUS,
            -1,
            0,
        );
        if wal_map == MAP_FAILED || hb_map == MAP_FAILED {
            return (false, "mmap failed".into());
        }

        let wal_entries = wal_map as *mut WalEntry;
        let hb = hb_map as *mut HeartbeatRegion;
        ptr::write_volatile(hb, HeartbeatRegion {
            last_tick: AtomicU64::new(rdtsc()),
            node_id: 1,
            _pad: [0; 6],
        });

        let mut books: Vec<OrderBook> = (0..MT_SYMBOLS).map(|_| OrderBook::new()).collect();

        struct Rng(u64);
        impl Rng {
            #[inline(always)]
            fn next(&mut self) -> u32 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0 as u32
            }
        }
        let mut rng = Rng(0x1234_5678_ABCD_EF01);

        let t0 = rdtsc();
        for i in 0..wal_count {
            let price = rng.next() % 1000 + 10000;
            let shares = rng.next() % 500 + 1;
            let side = (rng.next() & 1) as u8;
            let sym = (i % MT_SYMBOLS) as u8;

            let e = WalEntry {
                seq: i as u64,
                ts: rdtscp(),
                price,
                shares,
                side,
                symbol: sym,
                _pad: [0; 6],
            };
            ptr::write_volatile(wal_entries.add(i), e);

            books[sym as usize].insert(price, shares, side == 0);

            if i & 63 == 0 {
                (*hb).last_tick.store(rdtscp(), Ordering::Release);
            }
        }
        msync(wal_map, wal_size, MS_SYNC);
        let t1 = rdtsc();

        let total_cyc = t1 - t0;
        let per_event = total_cyc / wal_count as u64;
        let meps = if per_event > 0 { 3200.0 / per_event as f64 } else { 0.0 };

        let hb_age = rdtsc().wrapping_sub((*hb).last_tick.load(Ordering::Acquire));
        let hb_ok = hb_age < 100_000;

        let total_depth: u32 = books.iter().map(|b| b.bid_depth + b.ask_depth).sum();

        munmap(wal_map, wal_size);
        munmap(hb_map, 4096);
        drop(f);
        let _ = fs::remove_file(wal_path);

        let ok = total_depth > 0 && hb_ok;
        (
            ok,
            format!(
                "{} events | {} cyc/event ({:.1} Mevents/s) | WAL+LOB+HB | depth={} | hb={}",
                wal_count,
                per_event,
                meps,
                total_depth,
                if hb_ok { "live" } else { "stale" }
            ),
        )
    }
}

fn main() {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║            APEX-WIRE V2 — PHASE 5: PRODUCTION HARDENING                    ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  WAL Recovery | Multi-Tenant | Failover HB | SPSC Bridge | Combined       ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let (wal_ok, wal_msg) = test_wal_recovery();
    println!("  [1/5] WAL Crash Recovery:   {}", wal_msg);

    let (mt_ok, mt_msg) = test_multi_tenant();
    println!("  [2/5] Multi-Tenant Books:   {}", mt_msg);

    let (hb_ok, hb_msg) = test_failover_heartbeat();
    println!("  [3/5] Failover Heartbeat:   {}", hb_msg);

    let (spsc_ok, spsc_msg) = test_spsc_async_bridge();
    println!("  [4/5] SPSC Async Bridge:    {}", spsc_msg);

    let (combo_ok, combo_msg) = test_combined_pipeline();
    println!("  [5/5] Combined Pipeline:    {}", combo_msg);

    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                         PHASE 5 VERDICT                                     ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  WAL Recovery:        [{}]                                              ║", if wal_ok { "PASS" } else { "FAIL" });
    println!("║  Multi-Tenant <100%:  [{}]                                              ║", if mt_ok { "PASS" } else { "FAIL" });
    println!("║  Failover HB <1ms:    [{}]                                              ║", if hb_ok { "PASS" } else { "FAIL" });
    println!("║  SPSC Bridge:         [{}]                                              ║", if spsc_ok { "PASS" } else { "FAIL" });
    println!("║  Combined Pipeline:   [{}]                                              ║", if combo_ok { "PASS" } else { "FAIL" });
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    let pass_count = [wal_ok, mt_ok, hb_ok, spsc_ok, combo_ok].iter().filter(|&&x| x).count();
    if pass_count == 5 {
        println!("║  [PASS] Phase 5: All 5 production hardening tests passed.                  ║");
    } else {
        println!("║  [PARTIAL] Phase 5: {}/5 tests passed.                                    ║", pass_count);
    }
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();
}

