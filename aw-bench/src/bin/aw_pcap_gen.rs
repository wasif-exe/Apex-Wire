use std::fs::File;
use std::io::Write;

const PCAP_MAGIC_NS: u32 = 0xa1b23c4d;
const PCAP_SNAPLEN: u32 = 65535;
const NUM_MESSAGES: u32 = 500_000;

fn write_u16_le(w: &mut File, v: u16) { w.write_all(&v.to_le_bytes()).unwrap(); }
fn write_u32_le(w: &mut File, v: u32) { w.write_all(&v.to_le_bytes()).unwrap(); }
fn write_i32_le(w: &mut File, v: i32) { w.write_all(&v.to_le_bytes()).unwrap(); }

fn build_ether_ip_udp(payload: &[u8]) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let ip_len = 20 + udp_len;
    let eth_len = 14 + ip_len;
    let mut frame = vec![0u8; eth_len];
    frame[0..6].copy_from_slice(&[0x01, 0x00, 0x5e, 0x01, 0x02, 0x03]);
    frame[6..12].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
    frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    frame[14] = 0x45;
    frame[16..18].copy_from_slice(&(ip_len as u16).to_be_bytes());
    frame[23] = 17;
    frame[26..30].copy_from_slice(&[192, 168, 1, 1]);
    frame[30..34].copy_from_slice(&[224, 1, 2, 3]);
    frame[34..36].copy_from_slice(&12345u16.to_be_bytes());
    frame[36..38].copy_from_slice(&12345u16.to_be_bytes());
    frame[38..40].copy_from_slice(&(udp_len as u16).to_be_bytes());
    frame[42..42 + payload.len()].copy_from_slice(payload);
    frame
}

fn write_mold_packet(f: &mut File, session: &[u8; 10], seq: u64, msgs: &[Vec<u8>], ts: u64) {
    let mut mold = Vec::with_capacity(20 + msgs.len() * 40);
    mold.extend_from_slice(session);
    mold.extend_from_slice(&seq.to_be_bytes());
    mold.extend_from_slice(&(msgs.len() as u16).to_be_bytes());
    for m in msgs {
        mold.extend_from_slice(&(m.len() as u16).to_be_bytes());
        mold.extend_from_slice(m);
    }
    let frame = build_ether_ip_udp(&mold);
    let ts_sec = (ts / 1_000_000_000) as u32;
    let ts_nsec = (ts % 1_000_000_000) as u32;
    write_u32_le(f, ts_sec);
    write_u32_le(f, ts_nsec);
    write_u32_le(f, frame.len() as u32);
    write_u32_le(f, frame.len() as u32);
    f.write_all(&frame).unwrap();
}

fn make_add(oid: u64, price: u32, shares: u32, side: u8, ts: u64) -> Vec<u8> {
    let mut m = vec![0u8; 36];
    m[0] = b'A';
    m[1..7].copy_from_slice(&ts.to_be_bytes()[2..8]);
    m[7..15].copy_from_slice(&oid.to_be_bytes());
    m[15] = side;
    m[16..20].copy_from_slice(&1u32.to_be_bytes());
    m[20..28].copy_from_slice(b"AAPL    ");
    m[28..32].copy_from_slice(&shares.to_be_bytes());
    m[32..36].copy_from_slice(&price.to_be_bytes());
    m
}

fn make_exec(oid: u64, shares: u32, match_num: u64, ts: u64) -> Vec<u8> {
    let mut m = vec![0u8; 30];
    m[0] = b'E';
    m[1..7].copy_from_slice(&ts.to_be_bytes()[2..8]);
    m[7..15].copy_from_slice(&oid.to_be_bytes());
    m[15..19].copy_from_slice(&shares.to_be_bytes());
    m[19..27].copy_from_slice(&match_num.to_be_bytes());
    m
}

fn make_cancel(oid: u64, shares: u32, ts: u64) -> Vec<u8> {
    let mut m = vec![0u8; 22];
    m[0] = b'X';
    m[1..7].copy_from_slice(&ts.to_be_bytes()[2..8]);
    m[7..15].copy_from_slice(&oid.to_be_bytes());
    m[15..19].copy_from_slice(&shares.to_be_bytes());
    m
}

fn make_delete(oid: u64, ts: u64) -> Vec<u8> {
    let mut m = vec![0u8; 18];
    m[0] = b'D';
    m[1..7].copy_from_slice(&ts.to_be_bytes()[2..8]);
    m[7..15].copy_from_slice(&oid.to_be_bytes());
    m
}

fn make_system(ts: u64) -> Vec<u8> {
    let mut m = vec![0u8; 12];
    m[0] = b'S';
    m[1..7].copy_from_slice(&ts.to_be_bytes()[2..8]);
    m[7..11].copy_from_slice(&1u32.to_be_bytes());
    m[11] = b'O';
    m
}

fn main() {
    let path = "/tmp/aw_v4_test_itch.pcap";
    let mut f = File::create(path).unwrap();

    write_u32_le(&mut f, PCAP_MAGIC_NS);
    write_u16_le(&mut f, 2);
    write_u16_le(&mut f, 4);
    write_i32_le(&mut f, 0);
    write_i32_le(&mut f, 0);
    write_u32_le(&mut f, PCAP_SNAPLEN);
    write_u32_le(&mut f, 1);

    let session: [u8; 10] = *b"TESTSESS01";
    let mut seq: u64 = 1;
    let mut oid: u64 = 1;
    let mut match_num: u64 = 1;
    let mut ts: u64 = 34200_000_000_000;
    let mut pkt_count: u32 = 0;
    let base_price: u32 = 18000;

    write_mold_packet(&mut f, &session, seq, &[make_system(ts)], ts);
    pkt_count += 1;
    seq += 1;

    let mut batch: Vec<Vec<u8>> = Vec::new();
    for i in 0..NUM_MESSAGES {
        ts += 1000;
        let cycle = i % 10;
        let price = base_price + ((i % 20) as u32) * 5;
        let side = if i % 2 == 0 { b'B' } else { b'S' };

        let msg = match cycle {
            0 | 1 | 2 | 3 => {
                let m = make_add(oid, price, 100 + (i % 500) as u32, side, ts);
                oid += 1;
                m
            }
            4 | 5 => {
                let target = if oid > 2 { oid - 2 } else { 1 };
                make_exec(target, 50 + (i % 100) as u32, match_num, ts)
            }
            6 => {
                match_num += 1;
                let target = if oid > 3 { oid - 3 } else { 1 };
                make_cancel(target, 25, ts)
            }
            7 => {
                let target = if oid > 4 { oid - 4 } else { 1 };
                make_delete(target, ts)
            }
            _ => make_add(oid, price, 200, side, ts),
        };

        batch.push(msg);
        if batch.len() >= 10 || i == NUM_MESSAGES - 1 {
            write_mold_packet(&mut f, &session, seq, &batch, ts);
            pkt_count += 1;
            seq += 1;
            batch.clear();
        }
    }

    println!("generated {} ({} packets, {} ITCH messages)", path, pkt_count, NUM_MESSAGES);
}
