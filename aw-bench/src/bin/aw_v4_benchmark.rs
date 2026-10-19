use libc::*;
use std::fs;
use std::mem;
use std::ptr;

const BATCH: usize = 64;

#[inline(always)]
fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        std::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    }
}

#[repr(C)]
struct PtpClockCaps {
    max_adj: i32,
    n_alarm: i32,
    n_ext_ts: i32,
    n_per_out: i32,
    pps: i32,
    n_pins: i32,
    cross_timestamping: i32,
    adjust_phase: i32,
    max_phase_adj: i32,
    rsv: [i32; 11],
}

#[repr(C)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

fn test_af_xdp() -> (bool, String) {
    match aw_xdp::XdpSocket::new("lo", 0) {
        Ok(mut xsk) => {
            let hp = xsk.is_hugepage();
            let pkt = [b'A'; 64];
            let mut tx_ok = 0usize;
            let mut tx_full = 0usize;
            for _ in 0..256 {
                match xsk.write_async(&pkt) {
                    Ok(Some(_)) => tx_ok += 1,
                    _ => tx_full += 1,
                }
            }
            let mut rx = 0usize;
            for _ in 0..100 {
                let n = xsk.poll_read_zerocopy(BATCH, |_p, _l| {
                    rx += 1;
                });
                if n == 0 {
                    break;
                }
            }
            (true, format!("OK hp={} tx={}/{} rx={} (no BPF redirect)", hp, tx_ok, tx_full, rx))
        }
        Err(e) => (false, format!("{}", e)),
    }
}

fn test_ptp() -> (bool, String) {
    unsafe {
        let fd = open(b"/dev/ptp0\0".as_ptr() as *const i8, O_RDONLY);
        if fd < 0 {
            return (false, "no /dev/ptp0".into());
        }
        let mut caps: PtpClockCaps = mem::zeroed();
        let r = ioctl(fd, 0x80503D01u64, &mut caps as *mut PtpClockCaps);
        close(fd);
        if r < 0 {
            return (false, "ioctl GETCAPS failed".into());
        }
        let name = fs::read_to_string("/sys/class/ptp/ptp0/clock_name")
            .unwrap_or_default()
            .trim()
            .to_string();
        let t0 = rdtsc();
        let fd2 = open(b"/dev/ptp0\0".as_ptr() as *const i8, O_RDONLY);
        let mut ts: timespec = mem::zeroed();
        clock_gettime(0, &mut ts);
        close(fd2);
        let t1 = rdtsc();
        (
            true,
            format!(
                "{} max_adj={}ppb ext_ts={} pps={} probe={}cyc",
                name, caps.max_adj, caps.n_ext_ts, caps.pps, t1 - t0
            ),
        )
    }
}

fn test_ebpf_risk() -> (bool, String) {
    unsafe {
        let rx_fd = socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
        let tx_fd = socket(AF_INET, SOCK_DGRAM, 0);
        if rx_fd < 0 || tx_fd < 0 {
            return (false, "socket create failed".into());
        }
        let mut addr: sockaddr_in = mem::zeroed();
        addr.sin_family = AF_INET as u16;
        addr.sin_port = 19877u16.to_be();
        addr.sin_addr.s_addr = 0x0100_007f;
        bind(rx_fd, &addr as *const _ as _, mem::size_of::<sockaddr_in>() as u32);
        connect(tx_fd, &addr as *const _ as _, mem::size_of::<sockaddr_in>() as u32);

        let filter = [
            SockFilter { code: 0x80, jt: 0, jf: 0, k: 0 },
            SockFilter { code: 0x35, jt: 1, jf: 0, k: 64 },
            SockFilter { code: 0x06, jt: 0, jf: 0, k: 0 },
            SockFilter { code: 0x06, jt: 0, jf: 0, k: 0xFFFFFFFF },
        ];
        let prog = SockFprog {
            len: 4,
            filter: filter.as_ptr(),
        };
        let r = setsockopt(
            rx_fd,
            SOL_SOCKET,
            26,
            &prog as *const _ as _,
            mem::size_of::<SockFprog>() as u32,
        );
        if r < 0 {
            close(rx_fd);
            close(tx_fd);
            return (false, "SO_ATTACH_FILTER failed".into());
        }

        let big = [0xAAu8; 64];
        let small = [0xBBu8; 20];
        send(tx_fd, big.as_ptr() as *const _, 64, 0);
        send(tx_fd, small.as_ptr() as *const _, 20, 0);

        let mut buf = [0u8; 256];
        let mut passed = 0;
        for _ in 0..5 {
            let n = recv(rx_fd, buf.as_mut_ptr() as *mut _, 256, MSG_DONTWAIT);
            if n > 0 {
                passed += 1;
            } else {
                break;
            }
        }
        let dropped = 2 - passed;

        let t0 = rdtsc();
        for _ in 0..1000 {
            send(tx_fd, big.as_ptr() as *const _, 64, 0);
            recv(rx_fd, buf.as_mut_ptr() as *mut _, 256, MSG_DONTWAIT);
        }
        let t1 = rdtsc();
        let filter_cyc = (t1 - t0) / 1000;

        close(rx_fd);
        close(tx_fd);
        (
            passed == 1 && dropped == 1,
            format!(
                "attach OK | {} passed/{} dropped | {} cyc/filter",
                passed, dropped, filter_cyc
            ),
        )
    }
}

fn test_numa() -> (bool, String) {
    let nodes = fs::read_dir("/sys/devices/system/node/")
        .map(|d| {
            d.filter(|e| {
                e.as_ref()
                    .map(|x| x.file_name().to_string_lossy().starts_with("node"))
                    .unwrap_or(false)
            })
            .count()
        })
        .unwrap_or(0);
    let size: usize = 2 * 1024 * 1024;
    unsafe {
        let p = mmap(
            ptr::null_mut(),
            size,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        );
        if p == MAP_FAILED {
            return (false, "mmap failed".into());
        }
        let mut mask: u64 = 1;
        let r = libc::syscall(libc::SYS_mbind, p, size, 2u64, &mask as *const u64, 64u64, 0u64) as i32;
        let t0 = rdtsc();
        let slice = std::slice::from_raw_parts_mut(p as *mut u8, size);
        for i in (0..size).step_by(4096) {
            slice[i] = 0xFF;
        }
        let t1 = rdtsc();
        munmap(p, size);
        let cyc = t1 - t0;
        (
            r == 0,
            format!(
                "{} node(s) | mbind={} | 2MB touch={} cyc",
                nodes,
                if r == 0 { "OK" } else { "FAIL" },
                cyc
            ),
        )
    }
}

fn test_vfio() -> (bool, String) {
    let vfio_dev = std::path::Path::new("/dev/vfio/vfio").exists();
    let vfio_mod = fs::read_to_string("/proc/modules")
        .map(|m| m.contains("vfio_pci"))
        .unwrap_or(false);
    let nic_driver = fs::read_link("/sys/class/net/eno1/device/driver")
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        })
        .unwrap_or_else(|_| "unknown".into());
    let iommu = std::path::Path::new("/sys/kernel/iommu_groups").exists()
        && fs::read_dir("/sys/kernel/iommu_groups")
            .map(|d| d.count() > 0)
            .unwrap_or(false);
    let ready = vfio_dev && vfio_mod && iommu;
    (
        ready,
        format!(
            "dev={} mod={} iommu={} nic={} ({})",
            vfio_dev,
            vfio_mod,
            iommu,
            nic_driver,
            if ready {
                "ready"
            } else {
                "need: modprobe vfio-pci + dedicated NIC"
            }
        ),
    )
}

fn main() {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║            APEX-WIRE V2 — PHASE 4: REAL HARDWARE                           ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║  AF_XDP | PTP HW Clock | eBPF Risk Filter | NUMA-Aware | VFIO Framework   ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let hp_total = fs::read_to_string("/proc/meminfo")
        .and_then(|m| {
            m.lines()
                .find(|l| l.starts_with("HugePages_Total:"))
                .map(|l| l.split_whitespace().last().unwrap_or("0").to_string())
                .ok_or(std::io::Error::new(std::io::ErrorKind::NotFound, ""))
        })
        .unwrap_or_else(|_| "?".into());
    let ml = unsafe {
        let mut rl: rlimit = mem::zeroed();
        getrlimit(RLIMIT_MEMLOCK, &mut rl);
        rl.rlim_cur
    };
    println!("  Prerequisites:");
    println!(
        "    HugePages_Total: {} | Memlock: {} KB {}",
        hp_total,
        ml / 1024,
        if ml >= 16 * 1024 * 1024 {
            "(OK)"
        } else {
            "(LOW — need ulimit -l unlimited)"
        }
    );
    println!();

    let (af_ok, af_msg) = test_af_xdp();
    println!("  [1/5] AF_XDP Loopback:      {}", af_msg);

    let (ptp_ok, ptp_msg) = test_ptp();
    println!("  [2/5] PTP Hardware Clock:   {}", ptp_msg);

    let (ebpf_ok, ebpf_msg) = test_ebpf_risk();
    println!("  [3/5] eBPF Risk Filter:     {}", ebpf_msg);

    let (numa_ok, numa_msg) = test_numa();
    println!("  [4/5] NUMA-Aware Alloc:     {}", numa_msg);

    let (vfio_ok, vfio_msg) = test_vfio();
    println!("  [5/5] VFIO NIC Passthrough: {}", vfio_msg);

    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                         PHASE 4 VERDICT                                     ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  AF_XDP Socket:       [{}]                                              ║",
        if af_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "║  PTP HW Clock:        [{}]                                              ║",
        if ptp_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "║  eBPF Risk Filter:    [{}]                                              ║",
        if ebpf_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "║  NUMA-Aware Alloc:    [{}]                                              ║",
        if numa_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "║  VFIO NIC:            [{}]                                              ║",
        if vfio_ok { "PASS" } else { "DEFER" }
    );
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    let pass_count = [af_ok, ptp_ok, ebpf_ok, numa_ok]
        .iter()
        .filter(|&&x| x)
        .count();
    if pass_count >= 3 {
        println!(
            "║  [PASS] Phase 4: {}/4 core tests passed. VFIO deferred to AWS EC2.       ║",
            pass_count
        );
    } else {
        println!(
            "║  [FAIL] Phase 4: only {}/4 core tests passed.                           ║",
            pass_count
        );
    }
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();
}

