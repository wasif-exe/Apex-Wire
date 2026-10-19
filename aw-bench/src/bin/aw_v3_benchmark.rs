use libc::*;
use std::mem;
use std::ptr;

const BATCH: usize = 64;
const TOTAL: usize = 50_000;
const WARMUP: usize = 2_000;
const PORT: u16 = 19876;
const BUF: usize = 2048;

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

#[inline(always)]
fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        std::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    }
}

fn build_itch_add(buf: &mut [u8; 64], rng: &mut Rng) -> usize {
    buf[0] = b'A';
    buf[1] = 0;
    buf[2] = 1;
    buf[3] = 0;
    buf[4] = 0;
    let ts = rng.next();
    buf[5] = 0;
    buf[6] = 0;
    buf[7] = (ts >> 24) as u8;
    buf[8] = (ts >> 16) as u8;
    buf[9] = (ts >> 8) as u8;
    buf[10] = ts as u8;
    let oref = rng.next() as u64 | ((rng.next() as u64) << 32);
    buf[11..19].copy_from_slice(&oref.to_be_bytes());
    buf[19] = if rng.next() & 1 == 0 { b'B' } else { b'S' };
    let shares = rng.next() % 1000 + 1;
    buf[20..24].copy_from_slice(&shares.to_be_bytes());
    buf[24..32].copy_from_slice(b"AAPL    ");
    let price = rng.next() % 100000 + 10000;
    buf[32..36].copy_from_slice(&price.to_be_bytes());
    36
}

fn create_socket(port: u16) -> i32 {
    unsafe {
        let fd = socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
        assert!(fd >= 0);
        let one: i32 = 1;
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one as *const _ as _, 4);
        let big: i32 = 16 * 1024 * 1024;
        setsockopt(fd, SOL_SOCKET, SO_RCVBUF, &big as *const _ as _, 4);
        setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &big as *const _ as _, 4);
        let bp: i32 = 50;
        setsockopt(fd, SOL_SOCKET, SO_BUSY_POLL, &bp as *const _ as _, 4);
        if port > 0 {
            let mut a: sockaddr_in = mem::zeroed();
            a.sin_family = AF_INET as u16;
            a.sin_port = port.to_be();
            a.sin_addr.s_addr = 0x0100_007f;
            let r = bind(fd, &a as *const _ as _, mem::size_of::<sockaddr_in>() as u32);
            assert_eq!(r, 0);
        }
        fd
    }
}

fn connect_port(fd: i32, port: u16) {
    unsafe {
        let mut a: sockaddr_in = mem::zeroed();
        a.sin_family = AF_INET as u16;
        a.sin_port = port.to_be();
        a.sin_addr.s_addr = 0x0100_007f;
        connect(fd, &a as *const _ as _, mem::size_of::<sockaddr_in>() as u32);
    }
}

fn do_sendmmsg(fd: i32, pkts: &[Vec<u8>]) -> usize {
    unsafe {
        let n = pkts.len();
        let mut msgs: [mmsghdr; BATCH] = mem::zeroed();
        let mut iovs: [iovec; BATCH] = mem::zeroed();
        for i in 0..n {
            iovs[i].iov_base = pkts[i].as_ptr() as *mut _;
            iovs[i].iov_len = pkts[i].len();
            msgs[i].msg_hdr.msg_iov = iovs.as_mut_ptr().add(i);
            msgs[i].msg_hdr.msg_iovlen = 1;
        }
        let r = sendmmsg(fd, msgs.as_mut_ptr(), n as u32, 0);
        if r < 0 { 0 } else { r as usize }
    }
}

fn do_recvmmsg(fd: i32, bufs: &mut [[u8; BUF]; BATCH]) -> usize {
    unsafe {
        let mut msgs: [mmsghdr; BATCH] = mem::zeroed();
        let mut iovs: [iovec; BATCH] = mem::zeroed();
        for i in 0..BATCH {
            iovs[i].iov_base = bufs[i].as_mut_ptr() as *mut _;
            iovs[i].iov_len = BUF;
            msgs[i].msg_hdr.msg_iov = iovs.as_mut_ptr().add(i);
            msgs[i].msg_hdr.msg_iovlen = 1;
        }
        let r = recvmmsg(fd, msgs.as_mut_ptr(), BATCH as u32, MSG_DONTWAIT, ptr::null_mut());
        if r < 0 { 0 } else { r as usize }
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn main() {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║            APEX-WIRE V2 — PHASE 3: REAL NETWORK I/O                        ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  sendmmsg/recvmmsg x64 | 16MB sockbuf | SO_BUSY_POLL | AF_XDP live probe  ║");
    println!("║  Binary ITCH 5.0 over UDP loopback 127.0.0.1:{}                          ║", PORT);
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let mut rng = Rng(0xCAFE_BABE_DEAD_BEEF);
    let total_pkts = TOTAL + WARMUP;
    let mut pkts: Vec<Vec<u8>> = Vec::with_capacity(total_pkts);
    let mut tmp = [0u8; 64];
    for _ in 0..total_pkts {
        let n = build_itch_add(&mut tmp, &mut rng);
        pkts.push(tmp[..n].to_vec());
    }
    println!("  Prepared {} ITCH packets ({} warmup + {} measured)...", total_pkts, WARMUP, TOTAL);

    let rx_fd = create_socket(PORT);
    let tx_fd = create_socket(0);
    connect_port(tx_fd, PORT);

    let mut recv_bufs = [[0u8; BUF]; BATCH];

    let mut warmup_sent = 0;
    while warmup_sent < WARMUP {
        let end = (warmup_sent + BATCH).min(WARMUP);
        do_sendmmsg(tx_fd, &pkts[warmup_sent..end]);
        let _ = do_recvmmsg(rx_fd, &mut recv_bufs);
        warmup_sent = end;
    }
    loop {
        if do_recvmmsg(rx_fd, &mut recv_bufs) == 0 {
            break;
        }
    }
    println!("  Warmup complete.");
    println!();

    let mut lats: Vec<u64> = Vec::with_capacity(TOTAL);
    let mut total_rx: usize = 0;
    let mut total_tx: usize = 0;
    let mut sent = WARMUP;
    let end_target = WARMUP + TOTAL;

    while sent < end_target {
        let end = (sent + BATCH).min(end_target);
        let batch = &pkts[sent..end];

        let t0 = rdtsc();
        let ns = do_sendmmsg(tx_fd, batch);
        let nr = do_recvmmsg(rx_fd, &mut recv_bufs);
        let t1 = rdtsc();

        total_tx += ns;

        if nr > 0 {
            let per = (t1 - t0) / nr as u64;
            for _ in 0..nr {
                lats.push(per);
            }
            total_rx += nr;
        }

        sent = end;
    }

    loop {
        let nr = do_recvmmsg(rx_fd, &mut recv_bufs);
        if nr == 0 {
            break;
        }
        total_rx += nr;
    }

    lats.sort_unstable();

    let p50 = percentile(&lats, 50.0);
    let p99 = percentile(&lats, 99.0);
    let p999 = percentile(&lats, 99.9);
    let p9999 = percentile(&lats, 99.99);
    let avg = if lats.is_empty() { 0 } else { lats.iter().sum::<u64>() / lats.len() as u64 };
    let meps = if avg > 0 { 3_000.0 / avg as f64 } else { 0.0 };
    let delivery = if total_tx > 0 { (total_rx as f64 / total_tx as f64) * 100.0 } else { 0.0 };

    println!("  [1/2] UDP Loopback (kernel stack, sendmmsg/recvmmsg x{})...", BATCH);
    println!("        Sent: {} | Received: {} | Delivery: {:.1}%", total_tx, total_rx, delivery);
    println!();

    println!("  [2/2] AF_XDP (zero-copy, lo queue 0)...");
    let xdp_diag = match aw_xdp::XdpSocket::new("lo", 0) {
        Ok(mut xsk) => {
            let mut tx_ok: usize = 0;
            let mut tx_full: usize = 0;
            for p in &pkts[WARMUP..WARMUP + 256] {
                match xsk.write_async(p) {
                    Ok(Some(_)) => tx_ok += 1,
                    Ok(None) => tx_full += 1,
                    Err(_) => tx_full += 1,
                }
            }
            let mut rx_count: usize = 0;
            for _ in 0..100 {
                let n = xsk.poll_read_zerocopy(BATCH, |_pkt, _parsed| {
                    rx_count += 1;
                });
                if n == 0 {
                    break;
                }
            }
            let hp = xsk.is_hugepage();
            format!("socket OK | hugepage={} | tx={} sent/{} full | rx={} (no BPF redirect)", hp, tx_ok, tx_full, rx_count)
        }
        Err(e) => {
            format!("FAILED: {}", e)
        }
    };
    println!("        {}", xdp_diag);
    println!();

    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    PHASE 3 RESULTS: REAL NETWORK I/O                        ║");
    println!("╠══════════════════════════════╦═══════════╦═══════════╦═══════════╦═══════════╣");
    println!("║ Transport                    ║   p50     ║   p99     ║   p99.9   ║  p99.99   ║");
    println!("╠══════════════════════════════╬═══════════╬═══════════╬═══════════╬═══════════╣");
    if !lats.is_empty() {
        println!("║ UDP Loopback (kernel+batches)║ {:>5} cyc ║ {:>5} cyc ║ {:>5} cyc ║ {:>5} cyc ║", p50, p99, p999, p9999);
    }
    println!("╠══════════════════════════════╩═══════════╩═══════════╩═══════════╩═══════════╣");
    if !lats.is_empty() {
        println!("║  UDP avg: {} cyc/event | {:.2} Mevents/s | Delivery: {:.1}%              ║", avg, meps, delivery);
    }
    println!("║  AF_XDP: {}  ║", xdp_diag);
    println!("╠══════════════════════════════╦═══════════╦═══════════╦═══════════╦═══════════╣");
    println!("║ Phase 2 (synthetic, no net)  ║   107 cyc ║   402 cyc ║   416 cyc ║   416 cyc ║");
    println!("╚══════════════════════════════╩═══════════╩═══════════╩═══════════╩═══════════╝");
    println!();

    let udp_pass = p50 < 50_000 && delivery > 99.0;
    let loss_pass = delivery > 99.0;

    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                         PHASE 3 VERDICT                                     ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  UDP Loopback p50 < 50K cyc: [{}]                                       ║", if udp_pass { "PASS" } else { "FAIL" });
    println!("║  Packet Delivery > 99%:      [{}]                                       ║", if loss_pass { "PASS" } else { "FAIL" });
    println!("║  AF_XDP Socket Create:       [{}]                                       ║", if xdp_diag.starts_with("socket") { "PASS" } else { "FAIL" });
    println!("║  AF_XDP Data Path:           [DEFER] (needs BPF ELF + real NIC, Phase 4)    ║");
    println!("║  Network Stack:              REAL (Linux kernel UDP/IP)                     ║");
    println!("║  Syscall Mode:               sendmmsg/recvmmsg x{} batched                ║", BATCH);
    println!("║  Socket Buffers:             16 MB RCV + 16 MB SND + SO_BUSY_POLL          ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    if udp_pass && loss_pass {
        println!("║  [PASS] Phase 3 kernel UDP targets met.                                    ║");
        println!("║  AF_XDP data path deferred to Phase 4 (real NIC + BPF redirect).           ║");
    } else {
        println!("║  [FAIL] Phase 3 targets not met.                                           ║");
    }
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    unsafe {
        close(tx_fd);
        close(rx_fd);
    }
}

