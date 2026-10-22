use std::env;
use std::fs::File;
use std::io::Read;
use std::process;
use std::time::Instant;

use aw_exchange::itch::parse_itch_message;
use aw_exchange::lob::LimitOrderBook;
use aw_exchange::order::{EventKind, MarketEvent};

const USAGE: &str = "usage: aw_itch_replay <file.bin> [--max-msgs N]";

#[derive(Default)]
struct Stats {
    bytes_read: u64,
    messages_total: u64,
    itch_parsed: u64,
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
    info_stock_dir: u64,
    info_noii: u64,
    info_regsho: u64,
    info_mwcb: u64,
    info_ipo: u64,
    info_luld: u64,
    info_broken: u64,
    info_other: u64,
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
        _ => {}
    }
}

fn classify_info(type_byte: u8, stats: &mut Stats) {
    match type_byte {
        b'R' => stats.info_stock_dir += 1,
        b'I' => stats.info_noii += 1,
        b'Q' => stats.info_regsho += 1,
        b'V' | b'W' => stats.info_mwcb += 1,
        b'Y' => stats.info_ipo += 1,
        b'K' => stats.info_luld += 1,
        b'B' => stats.info_broken += 1,
        _ => stats.info_other += 1,
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("{}", USAGE);
        process::exit(1);
    }
    let path = &args[1];
    let mut max_msgs: u64 = 0;
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--max-msgs" && i + 1 < args.len() {
            max_msgs = args[i + 1].parse().unwrap_or(0);
            i += 2;
        } else {
            i += 1;
        }
    }

    println!();
    println!("  +=============================================================================+");
    println!("  |            A P E X - W I R E   V 4   I T C H   R E P L A Y                  |");
    println!("  |                                                                             |");
    println!("  |   Raw NASDAQ ITCH 5.0 binary file replay (sequential, single-threaded)      |");
    println!("  |   Format: [2-byte BE length][type byte][fields...] back to back             |");
    println!("  |   Hardware: Intel i5-4570 Haswell @ 3.2 GHz, single core pinned             |");
    println!("  +=============================================================================+");
    println!();

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[error] cannot open file: {}", e);
            process::exit(2);
        }
    };

    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    println!("  file              : {}", path);
    println!("  file size         : {} bytes ({:.1} MB)", file_size, file_size as f64 / (1024.0 * 1024.0));
    if max_msgs > 0 {
        println!("  max messages      : {}", max_msgs);
    }
    println!();

    let mut stats = Stats::default();
    let mut lob = LimitOrderBook::new(*b"REPLAY  ");
    let mut buf = vec![0u8; 1 << 20];
    let mut pos = 0;
    let mut valid = 0;

    let start = Instant::now();

    loop {
        if max_msgs > 0 && stats.messages_total >= max_msgs {
            break;
        }

        if valid - pos < 2 {
            if pos > 0 {
                buf.copy_within(pos..valid, 0);
                valid -= pos;
                pos = 0;
            }
            match file.read(&mut buf[valid..]) {
                Ok(0) => break,
                Ok(n) => {
                    valid += n;
                    stats.bytes_read += n as u64;
                }
                Err(e) => {
                    eprintln!("[error] read failed: {}", e);
                    break;
                }
            }
        }

        if valid - pos < 2 {
            break;
        }

        let msg_len = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
        if msg_len == 0 || msg_len > 200 {
            pos += 1;
            continue;
        }

        let total = 2 + msg_len;
        if valid - pos < total {
            if pos > 0 {
                buf.copy_within(pos..valid, 0);
                valid -= pos;
                pos = 0;
            }
            if buf.len() < total {
                buf.resize(total * 2, 0);
            }
            match file.read(&mut buf[valid..]) {
                Ok(0) => break,
                Ok(n) => {
                    valid += n;
                    stats.bytes_read += n as u64;
                }
                Err(_) => break,
            }
            if valid < total {
                break;
            }
        }

        let msg = &buf[pos + 2..pos + total];
        let type_byte = if msg.is_empty() { 0 } else { msg[0] };
        stats.messages_total += 1;
        pos += total;

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
        } else {
            classify_info(type_byte, &mut stats);
        }
    }

    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64();
    let bbo = lob.bbo();

    let info_total = stats.info_stock_dir
        + stats.info_noii
        + stats.info_regsho
        + stats.info_mwcb
        + stats.info_ipo
        + stats.info_luld
        + stats.info_broken
        + stats.info_other;
    let recognized_total = stats.itch_parsed + info_total;

    println!();
    println!("  +=============================================================================+");
    println!("  |                              R E S U L T S                                  |");
    println!("  +=============================================================================+");
    println!("  bytes_read          : {} ({:.1} MB)", stats.bytes_read, stats.bytes_read as f64 / (1024.0 * 1024.0));
    println!("  messages_total      : {}", stats.messages_total);
    println!("  order_events        : {} (parsed into MarketEvent + LOB)", stats.itch_parsed);
    println!("  info_messages       : {} (recognized, no LOB impact)", info_total);
    println!("  unrecognized        : {}", stats.messages_total - recognized_total);
    println!();
    println!("  +------------------------ ORDER EVENT BREAKDOWN ------------------------------+");
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
    println!();
    println!("  +------------------------ INFO MESSAGE BREAKDOWN -----------------------------+");
    println!("  StockDirectory (R)  : {}", stats.info_stock_dir);
    println!("  NOII (I)            : {}", stats.info_noii);
    println!("  RegSHO (Q)          : {}", stats.info_regsho);
    println!("  MWCB (V/W)          : {}", stats.info_mwcb);
    println!("  IPO (Y)             : {}", stats.info_ipo);
    println!("  LULD (K)            : {}", stats.info_luld);
    println!("  BrokenTrade (B)     : {}", stats.info_broken);
    println!("  Other info          : {}", stats.info_other);
    println!();
    println!("  +----------------------------- FINAL LOB -----------------------------------+");
    println!("  bid_price           : {} (single-book, all symbols mixed)", bbo.bid_price);
    println!("  bid_shares          : {}", bbo.bid_shares);
    println!("  ask_price           : {}", bbo.ask_price);
    println!("  ask_shares          : {}", bbo.ask_shares);
    println!();
    println!("  +---------------------------- PERFORMANCE ----------------------------------+");
    println!("  elapsed             : {:.3} s", secs);
    if secs > 0.0 {
        let mps = stats.messages_total as f64 / secs;
        let bps = stats.bytes_read as f64 / secs / (1024.0 * 1024.0);
        let order_mps = stats.itch_parsed as f64 / secs;
        println!("  throughput_total    : {:.1} M msg/s", mps / 1_000_000.0);
        println!("  throughput_order    : {:.1} M order evt/s", order_mps / 1_000_000.0);
        println!("  throughput_IO       : {:.1} MB/s", bps);
    }
    println!();

    let recognize_rate = if stats.messages_total > 0 {
        recognized_total as f64 / stats.messages_total as f64 * 100.0
    } else {
        0.0
    };
    println!("  recognition rate    : {:.1}% (order + info / total)", recognize_rate);
    println!("  +=============================================================================+");
    println!();
    if stats.messages_total == 0 {
        eprintln!("[error] read 0 messages from file");
        process::exit(3);
    }
    if stats.itch_parsed == 0 {
        eprintln!("[error] parsed 0 order events from {} messages", stats.messages_total);
        process::exit(4);
    }
}
