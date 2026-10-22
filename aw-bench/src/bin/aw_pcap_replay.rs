use std::env;
use std::process;
use std::time::Instant;

use aw_pcap::{PcapReader, parse_moldudp64_header, MoldUdp64Iter, MOLDUDP64_HEARTBEAT, TimestampPrecision};
use aw_exchange::itch::parse_itch_message;
use aw_exchange::order::{EventKind, MarketEvent};
use aw_exchange::lob::LimitOrderBook;

const USAGE: &str = "usage: aw_pcap_replay <path.pcap> [--snapshot-interval N]";

#[derive(Default)]
struct Stats {
    packets_total: u64,
    packets_udp: u64,
    packets_non_udp: u64,
    mold_sessions: u64,
    mold_heartbeats: u64,
    mold_messages: u64,
    itch_parsed: u64,
    itch_unknown: u64,
    bytes_scanned: u64,
    seq_gaps: u64,
    seq_last: u64,
    msg_add: u64,
    msg_add_mpid: u64,
    msg_executed: u64,
    msg_executed_price: u64,
    msg_cancel: u64,
    msg_delete: u64,
    msg_replace: u64,
    msg_trade: u64,
    msg_system: u64,
    msg_trading_action: u64,
    msg_other: u64,
}

fn classify(ev: &MarketEvent, stats: &mut Stats) {
    match ev.kind {
        EventKind::AddOrder => stats.msg_add += 1,
        EventKind::AddOrderMpid => stats.msg_add_mpid += 1,
        EventKind::OrderExecuted => stats.msg_executed += 1,
        EventKind::OrderExecutedWithPrice => stats.msg_executed_price += 1,
        EventKind::OrderCancel => stats.msg_cancel += 1,
        EventKind::OrderDelete => stats.msg_delete += 1,
        EventKind::OrderReplace => stats.msg_replace += 1,
        EventKind::Trade => stats.msg_trade += 1,
        EventKind::SystemEvent => stats.msg_system += 1,
        EventKind::TradingAction => stats.msg_trading_action += 1,
        _ => stats.msg_other += 1,
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("{}", USAGE);
        process::exit(1);
    }
    let path = &args[1];
    let mut snapshot_interval: u64 = 0;
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--snapshot-interval" && i + 1 < args.len() {
            snapshot_interval = args[i + 1].parse().unwrap_or(0);
            i += 2;
        } else {
            i += 1;
        }
    }

    println!();
    println!("  +=============================================================================+");
    println!("  |            A P E X - W I R E   V 4   P C A P   R E P L A Y                 |");
    println!("  |                                                                             |");
    println!("  |   Real NASDAQ ITCH 5.0 pcap validation harness                              |");
    println!("  |   Zero-copy mmap scan -> MoldUDP64 decode -> ITCH parse -> LOB apply        |");
    println!("  +=============================================================================+");
    println!();

    let mut reader = match PcapReader::open(path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[error] cannot open pcap: {}", e);
            process::exit(2);
        }
    };
    let info = reader.info();
    let precision_str = match info.precision {
        TimestampPrecision::Nanoseconds => "ns",
        TimestampPrecision::Microseconds => "us",
    };
    println!("  pcap file         : {}", path);
    println!("  precision         : {}", precision_str);
    println!("  snaplen           : {}", info.snaplen);
    println!("  linktype          : {}", info.linktype);
    println!("  packet_count      : {}", info.packet_count);
    println!();

    let mut stats = Stats::default();
    let mut lob = LimitOrderBook::new(*b"REPLAY  ");
    let mut session_label = [0u8; 10];

    let start = Instant::now();

    while let Some(pkt) = reader.next_packet() {
        stats.packets_total += 1;
        stats.bytes_scanned += pkt.data.len() as u64;

        let udp_payload = match aw_pcap::extract_udp_payload(pkt.data) {
            Some(p) => { stats.packets_udp += 1; p }
            None => { stats.packets_non_udp += 1; continue; }
        };

        let hdr = match parse_moldudp64_header(udp_payload) {
            Some(h) => h,
            None => continue,
        };

        if hdr.message_count == MOLDUDP64_HEARTBEAT {
            stats.mold_heartbeats += 1;
            continue;
        }

        if hdr.session != session_label {
            stats.mold_sessions += 1;
            session_label = hdr.session;
        }

        if stats.seq_last != 0 {
            let expected = stats.seq_last + stats.mold_messages;
            if expected > 0 && hdr.sequence_number != expected && hdr.sequence_number != 1 {
                stats.seq_gaps += 1;
            }
        }
        stats.seq_last = hdr.sequence_number;

        let iter = MoldUdp64Iter::new(udp_payload, hdr.message_count);
        for msg in iter {
            stats.mold_messages += 1;
            let mut ev = MarketEvent::default();
            if parse_itch_message(msg, &mut ev) {
                stats.itch_parsed += 1;
                classify(&ev, &mut stats);
                match ev.kind {
                    EventKind::AddOrder
                    | EventKind::AddOrderMpid
                    | EventKind::OrderExecuted
                    | EventKind::OrderExecutedWithPrice
                    | EventKind::OrderCancel
                    | EventKind::OrderDelete
                    | EventKind::OrderReplace => {
                        lob.apply_event(&ev);
                    }
                    _ => {}
                }
                if snapshot_interval > 0
                    && stats.itch_parsed % snapshot_interval == 0
                {
                    let bbo = lob.bbo();
                    println!("  [snapshot @ {} msgs] bid={} x {} | ask={} x {}",
                        stats.itch_parsed,
                        bbo.bid_price, bbo.bid_shares,
                        bbo.ask_price, bbo.ask_shares);
                }
            } else {
                stats.itch_unknown += 1;
            }
        }
    }

    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64();
    let bbo = lob.bbo();

    println!();
    println!("  +=============================================================================+");
    println!("  |                              R E S U L T S                                  |");
    println!("  +=============================================================================+");
    println!("  packets_total       : {}", stats.packets_total);
    println!("  packets_udp         : {}", stats.packets_udp);
    println!("  packets_non_udp     : {}", stats.packets_non_udp);
    println!("  mold_sessions       : {}", stats.mold_sessions);
    println!("  mold_heartbeats     : {}", stats.mold_heartbeats);
    println!("  mold_messages_total : {}", stats.mold_messages);
    println!("  itch_parsed_ok      : {}", stats.itch_parsed);
    println!("  itch_unknown_type   : {}", stats.itch_unknown);
    println!("  seq_gaps_detected   : {}", stats.seq_gaps);
    println!("  bytes_scanned       : {}", stats.bytes_scanned);
    println!();
    println!("  +---------------------------- ITCH BREAKDOWN -------------------------------+");
    println!("  AddOrder (A)        : {}", stats.msg_add);
    println!("  AddOrderMpid (F)    : {}", stats.msg_add_mpid);
    println!("  OrderExecuted (E)   : {}", stats.msg_executed);
    println!("  ExecutedWithPrice(C): {}", stats.msg_executed_price);
    println!("  OrderCancel (X)     : {}", stats.msg_cancel);
    println!("  OrderDelete (D)     : {}", stats.msg_delete);
    println!("  OrderReplace (U)    : {}", stats.msg_replace);
    println!("  Trade (P)           : {}", stats.msg_trade);
    println!("  SystemEvent (S)     : {}", stats.msg_system);
    println!("  TradingAction (H)   : {}", stats.msg_trading_action);
    println!("  Other               : {}", stats.msg_other);
    println!();
    println!("  +----------------------------- FINAL LOB -----------------------------------+");
    println!("  bid_price           : {}", bbo.bid_price);
    println!("  bid_shares          : {}", bbo.bid_shares);
    println!("  ask_price           : {}", bbo.ask_price);
    println!("  ask_shares          : {}", bbo.ask_shares);
    println!();
    println!("  +---------------------------- PERFORMANCE ----------------------------------+");
    println!("  elapsed             : {:.3} s", secs);
    if secs > 0.0 {
        let mps = stats.mold_messages as f64 / secs;
        let bps = stats.bytes_scanned as f64 / secs / (1024.0 * 1024.0);
        println!("  throughput_msgs/s   : {:.0}", mps);
        println!("  throughput_MB/s     : {:.1}", bps);
    }
    println!();
    println!("  +=============================================================================+");

    if stats.packets_total == 0 {
        process::exit(3);
    }
    if stats.itch_parsed == 0 && stats.mold_messages > 0 {
        eprintln!("[warn] parsed 0 ITCH messages from {} MoldUDP64 payloads", stats.mold_messages);
        process::exit(4);
    }
}
