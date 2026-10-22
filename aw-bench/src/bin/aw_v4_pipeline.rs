use aw_disruptor::Disruptor;
use aw_exchange::lob::LimitOrderBook;
use aw_exchange::order::{EventKind, MarketEvent, Side};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[inline(always)]
fn rdtsc() -> u64 {
    // SAFETY: _rdtsc is always available on x86_64.
    unsafe { core::arch::x86_64::_rdtsc() }
}

fn set_affinity(core: usize) {
    // SAFETY: cpu_set_t is zeroed then populated with a valid core id.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(core, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

#[inline(always)]
fn synthetic_event(seq: u64) -> MarketEvent {
    MarketEvent {
        timestamp_ns: seq * 100,
        order_id: seq + 1,
        price: 18000 + ((seq % 20) as u32) * 5,
        shares: 100 + ((seq % 10) as u32) * 100,
        kind: if seq % 3 == 0 {
            EventKind::OrderCancel
        } else {
            EventKind::AddOrder
        },
        side: if seq % 2 == 0 { Side::Buy } else { Side::Sell },
        symbol: *b"AAPL    ",
        ..Default::default()
    }
}

const RING_CAPACITY: usize = 65536;
const TOTAL_EVENTS: u64 = 10_000_000;
const DRAIN_BATCH: u64 = 256;

fn main() {
    println!("=== Apex-Wire V4 Pipeline Benchmark (Opt 3.5-4) ===");
    println!("Ring: {} slots ({} KB)", RING_CAPACITY, RING_CAPACITY * 64 / 1024);
    println!("Events: {}", TOTAL_EVENTS);
    println!("Drain batch: {}", DRAIN_BATCH);
    println!();

    let disruptor: &'static Disruptor<MarketEvent> = Box::leak(Box::new(
        Disruptor::<MarketEvent>::new(RING_CAPACITY)
            .expect("failed to allocate disruptor ring"),
    ));
    println!("Ring page kind: {:?}", disruptor.page_kind());

    let pub_handle = disruptor.publisher();
    let cons_handle = disruptor.consumer();

    let running = Arc::new(AtomicBool::new(false));
    let running_c = running.clone();

    let cons_thread = std::thread::spawn(move || {
        set_affinity(3);
        let mut cons = cons_handle;
        let mut lob = LimitOrderBook::new(*b"AAPL    ");
        let mut drain_latencies = Vec::with_capacity(256 * 1024);
        let mut total_apply_cyc: u64 = 0;
        let mut total_events: u64 = 0;
        let mut max_ring_used: u64 = 0;

        while !running_c.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }

        let t_start = rdtsc();

        while total_events < TOTAL_EVENTS {
            let avail = cons.available();
            let local = cons.next_seq();
            if avail >= local {
                let used = avail.wrapping_sub(local).wrapping_add(1);
                if used > max_ring_used {
                    max_ring_used = used;
                }
            }

            let t0 = rdtsc();
            let drained = cons.drain(DRAIN_BATCH, |ev| {
                let a0 = rdtsc();
                lob.apply_event(&ev);
                total_apply_cyc += rdtsc() - a0;
            });
            let t1 = rdtsc();

            if drained > 0 {
                drain_latencies.push(t1 - t0);
                total_events += drained;
            } else {
                std::hint::spin_loop();
            }
        }

        let t_end = rdtsc();
        (t_end - t_start, drain_latencies, total_apply_cyc, total_events, max_ring_used)
    });

    set_affinity(2);
    running.store(true, Ordering::Release);

    let mut pub_latencies = Vec::with_capacity(256 * 1024);
    let t_prod_start = rdtsc();

    for i in 0..TOTAL_EVENTS {
        let ev = synthetic_event(i);
        let t0 = rdtsc();
        let seq = pub_handle.claim_spin();
        *pub_handle.get_mut(seq) = ev;
        pub_handle.publish(seq);
        let t1 = rdtsc();
        pub_latencies.push(t1 - t0);
    }

    let t_prod_end = rdtsc();
    let prod_total = t_prod_end - t_prod_start;

    let (cons_total, drain_lats, apply_cyc, cons_events, max_ring) =
        cons_thread.join().unwrap();

    let elapsed_cyc = prod_total.max(cons_total);
    let ghz = 3.2;
    let elapsed_sec = elapsed_cyc as f64 / (ghz * 1e9);
    let throughput = TOTAL_EVENTS as f64 / elapsed_sec / 1e6;

    let avg_pub = pub_latencies.iter().sum::<u64>() / pub_latencies.len() as u64;

    let avg_drain = if drain_lats.is_empty() {
        0
    } else {
        drain_lats.iter().sum::<u64>() / drain_lats.len() as u64
    };
    let avg_apply = apply_cyc / cons_events;

    let mut sorted_drain = drain_lats.clone();
    sorted_drain.sort_unstable();
    let p50 = sorted_drain[sorted_drain.len() / 2];
    let p99 = sorted_drain[sorted_drain.len() * 99 / 100];
    let p999 = sorted_drain[sorted_drain.len() * 999 / 1000];

    let fill_pct = max_ring as f64 / RING_CAPACITY as f64 * 100.0;

    println!("--- Results ---");
    println!("Producer total:   {} cyc ({:.3} us)", prod_total, prod_total as f64 / (ghz * 1e3));
    println!("Consumer total:   {} cyc ({:.3} us)", cons_total, cons_total as f64 / (ghz * 1e3));
    println!("Throughput:       {:.2} M evt/sec", throughput);
    println!();
    println!("--- Per-Stage Latency ---");
    println!("Publish avg:      {} cyc ({:.1} ns)", avg_pub, avg_pub as f64 / ghz);
    println!("Drain avg:        {} cyc ({:.1} ns)", avg_drain, avg_drain as f64 / ghz);
    println!("Drain p50:        {} cyc", p50);
    println!("Drain p99:        {} cyc", p99);
    println!("Drain p99.9:      {} cyc", p999);
    println!("LOB apply avg:    {} cyc ({:.1} ns)", avg_apply, avg_apply as f64 / ghz);
    println!();
    println!("--- Ring Utilization ---");
    println!("Peak fill:        {} / {} ({:.1}%)", max_ring, RING_CAPACITY, fill_pct);
    println!();

    let pass = throughput > 20.0 && avg_pub < 500 && avg_apply < 2000;
    println!("PASS: {} (throughput > 20M, publish < 500 cyc, apply < 2000 cyc)", pass);
    if !pass {
        std::process::exit(1);
    }
}
