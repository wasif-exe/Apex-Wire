use std::fs::File;
use std::io::Write;

fn write_itch_msg(file: &mut File, msg: &[u8]) {
    let len = msg.len() as u16;
    file.write_all(&len.to_be_bytes()).unwrap();
    file.write_all(msg).unwrap();
}

fn main() {
    let path = "/tmp/aw_v4_test_itch_raw.bin";
    let mut file = File::create(path).unwrap();
    let mut count: u64 = 0;

    let system_msg = [
        b'S',
        0, 0, 0, 0, 0, 0, 0, 1,
        0, 0,
        b'O',
    ];
    write_itch_msg(&mut file, &system_msg);
    count += 1;

    let trading_action = [
        b'H',
        0, 0, 0, 0, 0, 0, 0, 2,
        0, 0,
        b'A', b'A', b'P', b'L', b' ', b' ', b' ', b' ',
        b'T',
    ];
    write_itch_msg(&mut file, &trading_action);
    count += 1;

    for i in 0..50_000u64 {
        let price = 18000u32 + ((i % 200) as u32) * 5;
        let shares = 100u32 + ((i % 50) as u32) * 100;
        let side = if i % 2 == 0 { b'B' } else { b'S' };
        let ref_num = i + 1;

        let add_order = [
            b'A',
            ((ref_num >> 56) & 0xFF) as u8,
            ((ref_num >> 48) & 0xFF) as u8,
            ((ref_num >> 40) & 0xFF) as u8,
            ((ref_num >> 32) & 0xFF) as u8,
            ((ref_num >> 24) & 0xFF) as u8,
            ((ref_num >> 16) & 0xFF) as u8,
            ((ref_num >> 8) & 0xFF) as u8,
            (ref_num & 0xFF) as u8,
            0, 0,
            side,
            ((shares >> 24) & 0xFF) as u8,
            ((shares >> 16) & 0xFF) as u8,
            ((shares >> 8) & 0xFF) as u8,
            (shares & 0xFF) as u8,
            b'A', b'A', b'P', b'L', b' ', b' ', b' ', b' ',
            ((price >> 24) & 0xFF) as u8,
            ((price >> 16) & 0xFF) as u8,
            ((price >> 8) & 0xFF) as u8,
            (price & 0xFF) as u8,
        ];
        write_itch_msg(&mut file, &add_order);
        count += 1;

        if i % 3 == 0 && i > 0 {
            let exec_shares = 50u32;
            let exec_ref = i;
            let match_num = i / 3;
            let executed = [
                b'E',
                ((exec_ref >> 56) & 0xFF) as u8,
                ((exec_ref >> 48) & 0xFF) as u8,
                ((exec_ref >> 40) & 0xFF) as u8,
                ((exec_ref >> 32) & 0xFF) as u8,
                ((exec_ref >> 24) & 0xFF) as u8,
                ((exec_ref >> 16) & 0xFF) as u8,
                ((exec_ref >> 8) & 0xFF) as u8,
                (exec_ref & 0xFF) as u8,
                ((exec_shares >> 24) & 0xFF) as u8,
                ((exec_shares >> 16) & 0xFF) as u8,
                ((exec_shares >> 8) & 0xFF) as u8,
                (exec_shares & 0xFF) as u8,
                ((match_num >> 56) & 0xFF) as u8,
                ((match_num >> 48) & 0xFF) as u8,
                ((match_num >> 40) & 0xFF) as u8,
                ((match_num >> 32) & 0xFF) as u8,
                ((match_num >> 24) & 0xFF) as u8,
                ((match_num >> 16) & 0xFF) as u8,
                ((match_num >> 8) & 0xFF) as u8,
                (match_num & 0xFF) as u8,
            ];
            write_itch_msg(&mut file, &executed);
            count += 1;
        }

        if i % 5 == 0 && i > 0 {
            let cancel_ref = i - 1;
            let cancel_shares = 100u32;
            let cancel = [
                b'X',
                ((cancel_ref >> 56) & 0xFF) as u8,
                ((cancel_ref >> 48) & 0xFF) as u8,
                ((cancel_ref >> 40) & 0xFF) as u8,
                ((cancel_ref >> 32) & 0xFF) as u8,
                ((cancel_ref >> 24) & 0xFF) as u8,
                ((cancel_ref >> 16) & 0xFF) as u8,
                ((cancel_ref >> 8) & 0xFF) as u8,
                (cancel_ref & 0xFF) as u8,
                ((cancel_shares >> 24) & 0xFF) as u8,
                ((cancel_shares >> 16) & 0xFF) as u8,
                ((cancel_shares >> 8) & 0xFF) as u8,
                (cancel_shares & 0xFF) as u8,
            ];
            write_itch_msg(&mut file, &cancel);
            count += 1;
        }

        if i % 7 == 0 && i > 0 {
            let del_ref = i - 2;
            let delete = [
                b'D',
                ((del_ref >> 56) & 0xFF) as u8,
                ((del_ref >> 48) & 0xFF) as u8,
                ((del_ref >> 40) & 0xFF) as u8,
                ((del_ref >> 32) & 0xFF) as u8,
                ((del_ref >> 24) & 0xFF) as u8,
                ((del_ref >> 16) & 0xFF) as u8,
                ((del_ref >> 8) & 0xFF) as u8,
                (del_ref & 0xFF) as u8,
            ];
            write_itch_msg(&mut file, &delete);
            count += 1;
        }
    }

    let file_size = file.metadata().unwrap().len();
    println!("Generated {} ITCH messages to {}", count, path);
    println!("File size: {} bytes ({:.1} MB)", file_size, file_size as f64 / (1024.0 * 1024.0));
}
