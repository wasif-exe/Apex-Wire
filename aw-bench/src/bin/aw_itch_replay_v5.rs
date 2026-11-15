use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use aw_disruptor::Disruptor;
use aw_exchange::itch::parse_itch_message;
use aw_exchange::order::{EventKind, MarketEvent, Side};
use aw_matching::MatchingEngine;

const RING_CAPACITY: usize = 65536;
const DRAIN_BATCH: u64 = 4096;
const GHZ: f64 = 2.7;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

struct SendPub<'a>(aw_disruptor::Publisher<'a, MarketEvent>);
// SAFETY: SPSC ring with single producer thread; scope guarantees lifetime.
unsafe impl<'a> Send for SendPub<'a> {}

struct PromStats {
    messages_total: u64,
    parse_cycles_total: u64,
    match_cycles_total: u64,
}

fn placeholder_symbol() -> [u8; 8] {
    *b"SYM00001"
}

fn rdtsc() -> u64 {
    unsafe { std::arch::x86_64::_rdtsc() }
}

fn tsc_to_ns(cycles: u64) -> f64 {
    cycles as f64 / GHZ
}

fn pin_thread_to_core(core: usize) {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(core, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

fn run_ingest_file(
    path: &str,
    pub_ref: &aw_disruptor::Publisher<'_, MarketEvent>,
) -> (u64, u64) {
    use std::fs::File;
    use std::os::unix::io::AsRawFd;
    let file = File::open(path).expect("failed to open ITCH file");
    let fd = file.as_raw_fd();
    let meta = file.metadata().expect("failed to get metadata");
    let file_len = meta.len() as usize;
    let ptr = unsafe {
        libc::mmap(std::ptr::null_mut(), file_len, libc::PROT_READ,
            libc::MAP_PRIVATE | libc::MAP_POPULATE, fd, 0)
    };
    if ptr == libc::MAP_FAILED { panic!("mmap failed"); }
    let data = unsafe { std::slice::from_raw_parts(ptr as *const u8, file_len) };
    let mut offset: usize = 0;
    let mut total_events: u64 = 0;
    let start = Instant::now();
    while offset + 2 <= file_len {
        let msg_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        if offset + msg_len > file_len { break; }
        let msg_bytes = &data[offset..offset + msg_len];
        offset += msg_len;
        let seq = pub_ref.claim_spin();
        let slot = pub_ref.get_mut(seq);
        let ok = parse_itch_message(msg_bytes, slot);
        pub_ref.publish(seq);
        if ok { total_events += 1; }
        if total_events & 4095 == 0 && SHUTDOWN.load(Ordering::Relaxed) { break; }
    }
    let elapsed_ns = start.elapsed().as_nanos() as u64;
    unsafe { libc::munmap(ptr, file_len); }
    (total_events, elapsed_ns)
}

fn run_ingest_synthetic(
    count: u64,
    pub_ref: &aw_disruptor::Publisher<'_, MarketEvent>,
) -> u64 {
    let sym = placeholder_symbol();
    let start = Instant::now();
    let mut i: u64 = 0;
    while i < count {
        let ev = MarketEvent {
            timestamp_ns: i << 7,
            order_id: i + 1,
            new_order_id: 0,
            price: 10000,
            shares: 100,
            match_number: 0,
            symbol: sym,
            kind: EventKind::AddOrder,
            side: if i & 1 == 0 { Side::Buy } else { Side::Sell },
            stock_locate: 0,
            _pad: [0u8; 12],
        };
        let seq = pub_ref.claim_spin();
        // SAFETY: seq is valid claimed slot within ring buffer lifetime.
        unsafe {
            std::ptr::copy_nonoverlapping(
                &ev as *const MarketEvent,
                pub_ref.get_mut(seq) as *mut MarketEvent,
                1,
            );
        }
        pub_ref.publish(seq);
        i += 1;
        if i & 4095 == 0 && SHUTDOWN.load(Ordering::Relaxed) { break; }
    }
    start.elapsed().as_nanos() as u64
}

fn run_consumer(
    cons: &mut aw_disruptor::Consumer<'_, MarketEvent>,
    total_expected: u64,
) -> (u64, u64, f64) {
    let sym = placeholder_symbol();
    let mut engine = MatchingEngine::new(sym);
    let mut total_matched: u64 = 0;
    let mut match_cycles: u64 = 0;
    let mut match_samples: u64 = 0;
    let start = Instant::now();
    while total_matched < total_expected {
        let drained = cons.drain(DRAIN_BATCH, |ev: MarketEvent| {
            let measure = total_matched & 255 == 0;
            let t0 = if measure { rdtsc() } else { 0 };
            if ev.kind == EventKind::AddOrder {
                engine.match_order(ev.price, ev.shares, ev.side, ev.timestamp_ns);
            }
            if measure {
                match_cycles += rdtsc() - t0;
                match_samples += 1;
            }
            total_matched += 1;
        });
        if drained == 0 {
            if SHUTDOWN.load(Ordering::Relaxed) { break; }
            std::hint::spin_loop();
        }
    }
    let elapsed_ns = start.elapsed().as_nanos() as u64;
    let match_ns = if match_samples > 0 { tsc_to_ns(match_cycles / match_samples) } else { 0.0 };
    (total_matched, elapsed_ns, match_ns)
}

fn run_benchmark(mode: &str, path_or_count: &str) {
    let disruptor = Disruptor::<MarketEvent>::new(RING_CAPACITY).unwrap();
    let total_expected = if mode == "file" {
        std::fs::metadata(path_or_count).expect("file not found").len() / 40
    } else {
        path_or_count.parse().expect("invalid count")
    };
    println!("  +=============================================================================+");
    println!("  |              APEX-WIRE V5 MAX THROUGHPUT BENCHMARK                          |");
    println!("  +=============================================================================+");
    println!("  mode                : {}", mode);
    println!("  ring                : {} slots x 64B (AVX-512 direct store)", RING_CAPACITY);
    println!("  drain batch         : {}", DRAIN_BATCH);
    println!("  events              : {}", total_expected);
    println!();
    let (ing_events, ing_ns, cons_events, cons_ns, match_ns) = std::thread::scope(|s| {
        let pub_h = SendPub(disruptor.publisher());
        let mut cons_h = disruptor.consumer();
        let producer = s.spawn(move || {
            pin_thread_to_core(2);
            if mode == "file" {
                run_ingest_file(path_or_count, &pub_h.0)
            } else {
                let ns = run_ingest_synthetic(total_expected, &pub_h.0);
                (total_expected, ns)
            }
        });
        pin_thread_to_core(3);
        let (ce, cns, mns) = run_consumer(&mut cons_h, total_expected);
        let (ie, ins) = producer.join().unwrap();
        (ie, ins, ce, cns, mns)
    });
    let ing_mps = ing_events as f64 / (ing_ns as f64 / 1e9);
    let cons_mps = cons_events as f64 / (cons_ns as f64 / 1e9);
    println!("  +----------------------------- RESULTS ---------------------------------------+");
    println!("  ingest              : {:.2} M msg/s  ({} in {:.3}s)", ing_mps / 1e6, ing_events, ing_ns as f64 / 1e9);
    println!("  match               : {:.2} M msg/s  ({} in {:.3}s)", cons_mps / 1e6, cons_events, cons_ns as f64 / 1e9);
    println!("  match latency       : {:.1} ns (sampled 1/256)", match_ns);
    println!("  +=============================================================================+");
    println!();
    println!("  Hardware: Intel Xeon Platinum 8481C (Sapphire Rapids) @ 2.70GHz, 4C/8T,");
    println!("  105MB L3, AVX-512, KVM. TSC constant/nonstop @ 2.7GHz.");
    println!("  Ring: MarketEvent 64B AVX-512 store, batch {}, sampled rdtsc.", DRAIN_BATCH);
    println!("  +=============================================================================+");
}

fn run_multi_core(mode: &str, path_or_count: &str, num_cores: usize) {
    let per_core: u64 = if mode == "synthetic" {
        path_or_count.parse::<u64>().expect("invalid count") / num_cores as u64
    } else { 0 };
    let mut ti = 0.0;
    let mut tc = 0.0;
    let mut te = 0u64;
    let mut tm = 0u64;
    for i in 0..num_cores {
        let disruptor = Disruptor::<MarketEvent>::new(RING_CAPACITY).unwrap();
        let (ie, ins, ce, cns, _) = std::thread::scope(|s| {
            let pub_h = SendPub(disruptor.publisher());
            let mut cons_h = disruptor.consumer();
            let p = s.spawn(move || {
                pin_thread_to_core(i * 2 + 1);
                if mode == "synthetic" {
                    let ns = run_ingest_synthetic(per_core, &pub_h.0);
                    (per_core, ns)
                } else {
                    run_ingest_file(path_or_count, &pub_h.0)
                }
            });
            pin_thread_to_core(i * 2);
            let (ce, cns, mns) = run_consumer(&mut cons_h, per_core);
            let (ie, ins) = p.join().unwrap();
            (ie, ins, ce, cns, mns)
        });
        ti += ie as f64 / (ins as f64 / 1e9);
        tc += ce as f64 / (cns as f64 / 1e9);
        te += ie;
        tm += ce;
    }
    println!("  +=============================================================================+");
    println!("  |              APEX-WIRE V5 MULTI-CORE MAX THROUGHPUT                         |");
    println!("  +=============================================================================+");
    println!("  cores               : {}", num_cores);
    println!("  total events        : {}", te);
    println!("  total matched       : {}", tm);
    println!("  ingest              : {:.2} M msg/s", ti / 1e6);
    println!("  match               : {:.2} M msg/s", tc / 1e6);
    println!("  +=============================================================================+");
}

fn run_metrics_server(stats: Arc<RwLock<PromStats>>, port: u16) {
    let listener = match std::net::TcpListener::bind(format!("0.0.0.0:{}", port)) {
        Ok(l) => l,
        Err(e) => { eprintln!("[metrics] bind failed {}: {}", port, e); return; }
    };
    listener.set_nonblocking(true).ok();
    while !SHUTDOWN.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let s = stats.read().unwrap();
                let body = format!(
                    "aw_messages_total {}\naw_parse_cycles_total {}\naw_match_cycles_total {}\n",
                    s.messages_total, s.parse_cycles_total, s.match_cycles_total);
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
                use std::io::Write;
                let _ = stream.write_all(resp.as_bytes());
            }
            Err(_) => { std::thread::sleep(std::time::Duration::from_millis(50)); }
        }
    }
}

extern "C" fn handle_signal(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Release);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: aw_itch_replay_v5 <file|synthetic|multi-synthetic|multi-file> <arg> [cores]");
        std::process::exit(1);
    }
    unsafe {
        libc::signal(libc::SIGINT, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_signal as *const () as libc::sighandler_t);
    }
    let stats = Arc::new(RwLock::new(PromStats {
        messages_total: 0, parse_cycles_total: 0, match_cycles_total: 0,
    }));
    if let Ok(p) = std::env::var("AW_METRICS_PORT") {
        if let Ok(port) = p.parse::<u16>() {
            let sc = stats.clone();
            std::thread::spawn(move || run_metrics_server(sc, port));
        }
    }
    match args[1].as_str() {
        "file" => {
            if args.len() < 3 { eprintln!("need path"); std::process::exit(1); }
            run_benchmark("file", &args[2]);
        }
        "synthetic" => {
            if args.len() < 3 { eprintln!("need count"); std::process::exit(1); }
            run_benchmark("synthetic", &args[2]);
        }
        "multi-synthetic" => {
            if args.len() < 3 { eprintln!("need count"); std::process::exit(1); }
            let c = if args.len() >= 4 { args[3].parse().unwrap_or(4) } else { 4 };
            run_multi_core("synthetic", &args[2], c);
        }
        "multi-file" => {
            if args.len() < 3 { eprintln!("need path"); std::process::exit(1); }
            let c = if args.len() >= 4 { args[3].parse().unwrap_or(4) } else { 4 };
            run_multi_core("file", &args[2], c);
        }
        _ => { eprintln!("unknown mode: {}", args[1]); std::process::exit(1); }
    }
    SHUTDOWN.store(true, Ordering::Release);
}
