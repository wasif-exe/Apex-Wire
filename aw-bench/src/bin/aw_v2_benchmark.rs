use aw_core::probes::rdtsc;
use aw_exchange::itch::parse_itch_message;
use aw_exchange::order::{EventKind, MarketEvent, Side};
use aw_exchange::lob::LimitOrderBook;
use aw_matching::MatchingEngine;
use aw_sor::router::SmartOrderRouter;
use aw_pmc::PmcProfiler;

const SYMBOL: [u8; 8] = *b"AAPL    ";
const NUM_RESTING: usize = 10_000;
const NUM_EVENTS: usize = 100_000;
const WARMUP: usize = 2_000;
const BATCH_SIZE: usize = 1_000;
const BID_BASE: u32 = 18000;
const BID_TOP: u32 = 18249;
const ASK_BASE: u32 = 18251;
const ASK_TOP: u32 = 18500;

struct Rng(u64);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as u32
    }

    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next_u32() % (hi - lo + 1))
    }
}

struct OrderRef {
    id: u32,
    _gen: u16,
}

enum Decision {
    PlaceLimit,
    Cancel,
    Hold,
}

struct DecisionKernel {
    target_spread: u32,
    placed: u64,
    cancelled: u64,
    held: u64,
}

impl DecisionKernel {
    fn new(target_spread: u32) -> Self {
        Self { target_spread, placed: 0, cancelled: 0, held: 0 }
    }

    fn decide(&mut self, spread: u32, fill_count: u32) -> Decision {
        if fill_count > 0 {
            self.placed += 1;
            Decision::PlaceLimit
        } else if spread > self.target_spread {
            self.placed += 1;
            Decision::PlaceLimit
        } else if spread == 0 {
            self.cancelled += 1;
            Decision::Cancel
        } else {
            self.held += 1;
            Decision::Hold
        }
    }
}

fn build_itch_add(buf: &mut [u8; 64], ref_id: u64, side: u8, shares: u32, price: u32, ts: u64) -> usize {
    buf[0] = b'A';
    buf[1..3].copy_from_slice(&1u16.to_be_bytes());
    buf[3..5].copy_from_slice(&0u16.to_be_bytes());
    let tb = ts.to_be_bytes();
    buf[5..11].copy_from_slice(&tb[2..8]);
    buf[11..19].copy_from_slice(&ref_id.to_be_bytes());
    buf[19] = side;
    buf[20..24].copy_from_slice(&shares.to_be_bytes());
    buf[24..32].copy_from_slice(&SYMBOL);
    buf[32..36].copy_from_slice(&price.to_be_bytes());
    36
}

fn build_itch_cancel(buf: &mut [u8; 64], ref_id: u64, shares: u32, ts: u64) -> usize {
    buf[0] = b'X';
    buf[1..3].copy_from_slice(&1u16.to_be_bytes());
    buf[3..5].copy_from_slice(&0u16.to_be_bytes());
    let tb = ts.to_be_bytes();
    buf[5..11].copy_from_slice(&tb[2..8]);
    buf[11..19].copy_from_slice(&ref_id.to_be_bytes());
    buf[19..23].copy_from_slice(&shares.to_be_bytes());
    23
}

fn build_itch_exec(buf: &mut [u8; 64], ref_id: u64, shares: u32, match_num: u64, ts: u64) -> usize {
    buf[0] = b'E';
    buf[1..3].copy_from_slice(&1u16.to_be_bytes());
    buf[3..5].copy_from_slice(&0u16.to_be_bytes());
    let tb = ts.to_be_bytes();
    buf[5..11].copy_from_slice(&tb[2..8]);
    buf[11..19].copy_from_slice(&ref_id.to_be_bytes());
    buf[19..23].copy_from_slice(&shares.to_be_bytes());
    buf[23..31].copy_from_slice(&match_num.to_be_bytes());
    31
}

struct PreparedEvent {
    itch_buf: [u8; 64],
    itch_len: usize,
}

fn prepare_events(count: usize, rng: &mut Rng, bid_orders: &[OrderRef], ask_orders: &[OrderRef]) -> Vec<PreparedEvent> {
    let mut events = Vec::with_capacity(count);
    let mut itch_buf = [0u8; 64];
    let mut ts: u64 = 1_000_000;
    let mut next_ref_id: u64 = 100_000;
    let mut match_num: u64 = 1;

    for _ in 0..count {
        ts += 100;
        let roll = rng.next_u32() % 20;
        let len = if roll < 12 {
            let price = rng.range(BID_TOP - 10, ASK_BASE + 10);
            let side = if rng.next_u32() % 2 == 0 { b'B' } else { b'S' };
            build_itch_add(&mut itch_buf, next_ref_id, side, rng.range(10, 500), price, ts)
        } else if roll < 16 {
            let target = if !bid_orders.is_empty() {
                &bid_orders[rng.next_u32() as usize % bid_orders.len()]
            } else {
                &ask_orders[rng.next_u32() as usize % ask_orders.len()]
            };
            build_itch_cancel(&mut itch_buf, target.id as u64, 100, ts)
        } else if roll < 19 {
            let target = if !ask_orders.is_empty() {
                &ask_orders[rng.next_u32() as usize % ask_orders.len()]
            } else {
                &bid_orders[rng.next_u32() as usize % bid_orders.len()]
            };
            match_num += 1;
            build_itch_exec(&mut itch_buf, target.id as u64, 50, match_num, ts)
        } else {
            let price = rng.range(BID_TOP - 5, ASK_BASE + 5);
            let side = if rng.next_u32() % 2 == 0 { b'B' } else { b'S' };
            build_itch_add(&mut itch_buf, next_ref_id, side, rng.range(10, 200), price, ts)
        };
        next_ref_id += 1;
        events.push(PreparedEvent { itch_buf, itch_len: len });
    }
    events
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() { return 0; }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn main() {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║          APEX-WIRE V2 — PHASE 2: REALISTIC PIPELINE SIMULATION              ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  Config: {} resting orders | Binary ITCH | Real Decision Kernel        ║", NUM_RESTING);
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let mut engine = MatchingEngine::new(SYMBOL);
    let mut lob = LimitOrderBook::new(SYMBOL);
    let mut sor = SmartOrderRouter::new(SYMBOL);
    let mut kernel = DecisionKernel::new(4);
    let mut rng = Rng(0xDEAD_BEEF_CAFE_1234);

    sor.update_venue_bbo(0, BID_TOP, 1000, ASK_BASE, 1000);
    sor.update_venue_bbo(1, BID_TOP - 1, 500, ASK_BASE + 1, 500);

    let mut bid_orders: Vec<OrderRef> = Vec::with_capacity(NUM_RESTING / 2);
    let mut ask_orders: Vec<OrderRef> = Vec::with_capacity(NUM_RESTING / 2);

    print!("  Pre-loading {} resting orders... ", NUM_RESTING);
    let t_pre = rdtsc();

    for i in 0..(NUM_RESTING / 2) {
        let price = BID_BASE + (i as u32 % (BID_TOP - BID_BASE + 1));
        let r = engine.match_order(price, 100, Side::Buy, i as u64 * 100);
        let mut ev = MarketEvent::default();
        ev.kind = EventKind::AddOrder;
        ev.price = price;
        ev.shares = 100;
        ev.side = Side::Buy;
        lob.apply_event(&ev);
        if r.resting_order_id != u32::MAX {
            bid_orders.push(OrderRef { id: r.resting_order_id, _gen: r.resting_generation });
        }
    }

    for i in 0..(NUM_RESTING / 2) {
        let price = ASK_BASE + (i as u32 % (ASK_TOP - ASK_BASE + 1));
        let r = engine.match_order(price, 100, Side::Sell, (i + 5000) as u64 * 100);
        let mut ev = MarketEvent::default();
        ev.kind = EventKind::AddOrder;
        ev.price = price;
        ev.shares = 100;
        ev.side = Side::Sell;
        lob.apply_event(&ev);
        if r.resting_order_id != u32::MAX {
            ask_orders.push(OrderRef { id: r.resting_order_id, _gen: r.resting_generation });
        }
    }

    let pre_cycles = rdtsc().wrapping_sub(t_pre);
    let bbo = engine.bbo();
    println!("done ({} cyc)", pre_cycles);
    println!("  BBO: {} / {} (spread: {})", bbo.bid_price, bbo.ask_price, bbo.spread);
    println!("  Bid orders resting: {} | Ask orders resting: {}", bid_orders.len(), ask_orders.len());
    println!();

    print!("  Preparing {} binary ITCH events... ", NUM_EVENTS + WARMUP);
    let prepared = prepare_events(NUM_EVENTS + WARMUP, &mut rng, &bid_orders, &ask_orders);
    println!("done");
    println!();

    let mut event = MarketEvent::default();
    let mut total_fills: u64 = 0;

    print!("  Warming up ({} events)... ", WARMUP);
    for pe in &prepared[..WARMUP] {
        let _ = parse_itch_message(&pe.itch_buf[..pe.itch_len], &mut event);
        let risk_pass = event.price > 0 && event.shares > 0 && event.shares <= 1_000_000;
        if risk_pass {
            lob.apply_event(&event);
            let mr = engine.match_order(event.price, event.shares, event.side, event.timestamp_ns);
            let _ = sor.route_order(event.side, event.shares);
            let bbo_now = engine.bbo();
            let _ = kernel.decide(bbo_now.spread, mr.fill_count);
            total_fills += mr.fill_count as u64;
        }
    }
    println!("done");
    println!();

    let pmc = PmcProfiler::new();
    let pmc_before = pmc.snapshot();

    let mut batch_lats = Vec::with_capacity(NUM_EVENTS / BATCH_SIZE);
    let num_batches = NUM_EVENTS / BATCH_SIZE;
    let start_idx = WARMUP;

    print!("  Running realistic pipeline ({} events, {} batches of {})... ", NUM_EVENTS, num_batches, BATCH_SIZE);

    for b in 0..num_batches {
        let batch_start = start_idx + b * BATCH_SIZE;
        let batch_end = batch_start + BATCH_SIZE;

        let t0 = rdtsc();
        for pe in &prepared[batch_start..batch_end] {
            let parsed = parse_itch_message(&pe.itch_buf[..pe.itch_len], &mut event);
            let risk_pass = parsed && event.price > 0 && event.shares > 0 && event.shares <= 1_000_000;
            if risk_pass {
                lob.apply_event(&event);
                let mr = engine.match_order(event.price, event.shares, event.side, event.timestamp_ns);
                let _ = sor.route_order(event.side, event.shares);
                let bbo_now = engine.bbo();
                let _ = kernel.decide(bbo_now.spread, mr.fill_count);
                total_fills += mr.fill_count as u64;
            }
        }
        let t1 = rdtsc();
        batch_lats.push(t1.wrapping_sub(t0));
    }

    let pmc_after = pmc.snapshot();
    println!("done");
    println!();

    batch_lats.sort_unstable();
    let per_event_p50 = percentile(&batch_lats, 50.0) / BATCH_SIZE as u64;
    let per_event_p99 = percentile(&batch_lats, 99.0) / BATCH_SIZE as u64;
    let per_event_p999 = percentile(&batch_lats, 99.9) / BATCH_SIZE as u64;
    let per_event_p9999 = percentile(&batch_lats, 99.99) / BATCH_SIZE as u64;
    let total_cyc = batch_lats.iter().sum::<u64>();
    let avg_cyc = total_cyc / NUM_EVENTS as u64;

    println!("  Calibrating per-stage latency (isolated runs)...");

    let mut parse_cyc = 0u64;
    let cal_count = 10_000;
    let t0 = rdtsc();
    for pe in &prepared[..cal_count] {
        let _ = parse_itch_message(&pe.itch_buf[..pe.itch_len], &mut event);
    }
    parse_cyc = rdtsc().wrapping_sub(t0) / cal_count as u64;

    let mut risk_cyc = 0u64;
    let t0 = rdtsc();
    for pe in &prepared[..cal_count] {
        let _ = parse_itch_message(&pe.itch_buf[..pe.itch_len], &mut event);
        let _ = event.price > 0 && event.shares > 0 && event.shares <= 1_000_000;
    }
    risk_cyc = rdtsc().wrapping_sub(t0) / cal_count as u64;
    risk_cyc = risk_cyc.saturating_sub(parse_cyc);

    let mut lob_cyc = 0u64;
    let t0 = rdtsc();
    for pe in &prepared[..cal_count] {
        let _ = parse_itch_message(&pe.itch_buf[..pe.itch_len], &mut event);
        lob.apply_event(&event);
    }
    lob_cyc = rdtsc().wrapping_sub(t0) / cal_count as u64;
    lob_cyc = lob_cyc.saturating_sub(parse_cyc);

    let mut match_cyc = 0u64;
    let t0 = rdtsc();
    for pe in &prepared[..cal_count] {
        let _ = parse_itch_message(&pe.itch_buf[..pe.itch_len], &mut event);
        let _ = engine.match_order(event.price, event.shares, event.side, event.timestamp_ns);
    }
    match_cyc = rdtsc().wrapping_sub(t0) / cal_count as u64;
    match_cyc = match_cyc.saturating_sub(parse_cyc);

    let mut sor_cyc = 0u64;
    let t0 = rdtsc();
    for pe in &prepared[..cal_count] {
        let _ = parse_itch_message(&pe.itch_buf[..pe.itch_len], &mut event);
        let _ = sor.route_order(event.side, event.shares);
    }
    sor_cyc = rdtsc().wrapping_sub(t0) / cal_count as u64;
    sor_cyc = sor_cyc.saturating_sub(parse_cyc);

    let mut decision_cyc = 0u64;
    let t0 = rdtsc();
    for _ in 0..cal_count {
        let bbo_now = engine.bbo();
        let _ = kernel.decide(bbo_now.spread, 0);
    }
    decision_cyc = rdtsc().wrapping_sub(t0) / cal_count as u64;

    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    PER-STAGE LATENCY (calibrated, 10K book)                 ║");
    println!("╠══════════════════════════════╦═══════════════════════════════════════════════╣");
    println!("║ Stage                        ║  avg cyc (isolated calibration)              ║");
    println!("╠══════════════════════════════╬═══════════════════════════════════════════════╣");
    println!("║ ITCH Parse (binary)          ║  {:>5} cyc                                  ║", parse_cyc);
    println!("║ Risk Check                   ║  {:>5} cyc                                  ║", risk_cyc);
    println!("║ LOB Update                   ║  {:>5} cyc                                  ║", lob_cyc);
    println!("║ Match Engine (10K resting)   ║  {:>5} cyc                                  ║", match_cyc);
    println!("║ SOR Route                    ║  {:>5} cyc                                  ║", sor_cyc);
    println!("║ Decision Kernel              ║  {:>5} cyc                                  ║", decision_cyc);
    println!("╠══════════════════════════════╬═══════════════════════════════════════════════╣");
    println!("║ Sum of stages                ║  {:>5} cyc                                  ║",
        parse_cyc + risk_cyc + lob_cyc + match_cyc + sor_cyc + decision_cyc);
    println!("╚══════════════════════════════╩═══════════════════════════════════════════════╝");
    println!();

    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║               E2E BATCH LATENCY ({} events/batch, no rdtsc overhead)     ║", BATCH_SIZE);
    println!("╠══════════════════════════════╦═══════════╦═══════════╦═══════════╦═══════════╣");
    println!("║ Metric                       ║   p50     ║   p99     ║   p99.9   ║  p99.99   ║");
    println!("╠══════════════════════════════╬═══════════╬═══════════╬═══════════╬═══════════╣");
    println!("║ Per-event (batch-avg)        ║ {:>5} cyc ║ {:>5} cyc ║ {:>5} cyc ║ {:>5} cyc ║",
        per_event_p50, per_event_p99, per_event_p999, per_event_p9999);
    println!("╠══════════════════════════════╩═══════════╩═══════════╩═══════════╩═══════════╣");
    println!("║  Aggregate: {} cyc/event avg | {:.2} Mevents/s                            ║",
        avg_cyc,
        (NUM_EVENTS as f64 / (total_cyc as f64 / 3_000_000_000.0)) / 1_000_000.0);
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                         PMC HARDWARE COUNTERS                               ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  PMC Status: {}                                                   ║", pmc.status_label());
    println!("║  Cycles (PMC):     {:>12}                                               ║",
        pmc_after.cycles.wrapping_sub(pmc_before.cycles));
    println!("║  Instructions:     {:>12}                                               ║",
        pmc_after.instructions.wrapping_sub(pmc_before.instructions));
    println!("║  LLC Misses:       {:>12}                                               ║",
        pmc_after.llc_misses.wrapping_sub(pmc_before.llc_misses));
    println!("║  Branch Misses:    {:>12}                                               ║",
        pmc_after.branch_misses.wrapping_sub(pmc_before.branch_misses));
    let pmc_cyc = pmc_after.cycles.wrapping_sub(pmc_before.cycles);
    let pmc_ins = pmc_after.instructions.wrapping_sub(pmc_before.instructions);
    let ipc = if pmc_cyc > 0 { pmc_ins as f64 / pmc_cyc as f64 } else { 0.0 };
    println!("║  IPC:              {:>12.2}                                               ║", ipc);
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                        DECISION KERNEL STATS                                ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  Orders Placed:    {:>10}                                                  ║", kernel.placed);
    println!("║  Orders Cancelled: {:>10}                                                  ║", kernel.cancelled);
    println!("║  Held (no action): {:>10}                                                  ║", kernel.held);
    println!("║  Total Fills:      {:>10}                                                  ║", total_fills);
    println!("║  Mode:             REAL (spread-based market making)                        ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let pass_e2e = per_event_p50 < 200;
    let pass_parse = parse_cyc < 10;
    let pass_match = match_cyc < 80;
    let pass_all = pass_e2e && pass_parse && pass_match;

    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                         PHASE 2 VERDICT                                     ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  E2E p50 < 200 cyc:     {} ({:>5} cyc)                                    ║",
        if pass_e2e { "[PASS]" } else { "[FAIL]" }, per_event_p50);
    println!("║  ITCH Parse < 10 cyc:   {} ({:>5} cyc)                                    ║",
        if pass_parse { "[PASS]" } else { "[FAIL]" }, parse_cyc);
    println!("║  Match < 80 cyc:        {} ({:>5} cyc)                                    ║",
        if pass_match { "[PASS]" } else { "[FAIL]" }, match_cyc);
    println!("║  E2E p99:               {:>5} cyc                                          ║", per_event_p99);
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    if pass_all {
        println!("║  [PASS] Phase 2 realistic pipeline simulation verified.                     ║");
    } else {
        println!("║  [FAIL] Some Phase 2 targets not met.                                       ║");
    }
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();
}

