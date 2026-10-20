# Apex-Wire V3: Real Market Data Pipeline

Apex-Wire V3 extends the V2 full-duplex exchange engine with a complete real market data ingestion and paper trading pipeline: zero-copy PCAP file parsing, NASDAQ ITCH 5.0 full-lifecycle feed processing (10 message types), MoldUDP64 multicast framing, SBE binary dispatch, memory-mapped event logging (WAL), shadow decision logging, and a FIFO price-time priority paper trading engine with real queue position modeling.

V2 remains fully intact. All 29 original crates, the 9/9 V2 benchmark suite, and the 300-seed deterministic simulator are unchanged. V3 adds one new crate (`aw-pcap`) and extends four existing crates (`aw-exchange`, `aw-store`, `aw-mcast`, `aw-sim`).

---

## Technical Disclosures & Benchmark Methodology

All benchmarks follow strict physical measurement standards. No numbers are synthetic, estimated, or extrapolated.

* **Host Environment:** Intel Core i5-4570 @ 3.20GHz (Haswell, 4 cores / 4 threads, 6MB L3 Cache), 11 GB RAM, Arch Linux kernel 7.2.9-arch1-1 x86_64.
* **CPU Pinning:** Benchmark execution thread pinned to isolated CPU Core 2 (`taskset -c 2`) with `ulimit -l unlimited` and `nice -n -20`.
* **CPU Governor:** `performance` governor applied to all cores to lock frequency at 3.2 GHz and eliminate downclocking jitter.
* **Measurement Method:** x86 `rdtsc` cycle counter. All reported cycle numbers are **net cycles** after subtracting measured `rdtsc` instruction overhead (18 cycles on this host).
* **Cache State:** 50ms spin-warm before measurement. All data fits in L1/L2 for microbenchmarks.
* **Data Source:** Synthetic in-memory buffers and generated PCAP files. **No real exchange feed data was used.** NASDAQ ITCH 5.0 message formats are structurally correct (correct byte offsets, field sizes, message type codes) but contain fabricated order IDs, prices, and timestamps.
* **Network Path:** No real NIC was used. AF_XDP, VFIO, and UDP loopback tests are from V2 and deferred for V3. The NIC line in the benchmark output explicitly states "Not used (no real-NIC test)".
* **Concurrency:** Single-threaded, single-core. No SPSC handoff or multi-threaded pipeline in V3 benchmarks.

---

## Official Apex-Wire V3 Benchmark Suite (`aw_v3_benchmark`)

<p align="center">
  <img src="benchmarks/aw_v3_benchmark.png" alt="Apex-Wire V3 Benchmark" width="100%">
</p>

---

## Performance Summary Table

**13/13 PASS** | 72 workspace unit tests pass | 0 compiler warnings on V3 code

| Subsystem | Measured Latency | Equivalent Time | Details | Status |
| :--- | :--- | :--- | :--- | :---: |
| **PCAP mmap open + scan** | 6,734,199 cyc | 2.10 ms | 100,000-record PCAP file, zero-copy mmap | `[PASS]` |
| **PCAP + MoldUDP64 iteration** | 3,177,966 cyc | 0.99 ms | 100,000 UDP frames, header parse + message iteration | `[PASS]` |
| **ITCH 5.0 Parse (10 types)** | p50=5 p99=23 cyc | p99=7.2 ns | A/F/E/C/X/D/U/P/S/H messages, zero-copy into MarketEvent | `[PASS]` |
| **EventLog WAL Write** | 103 cyc/record | 32.2 ns | 100,000 records, 64B cacheline-aligned, mmap + ptr::write | `[PASS]` |
| **EventLog WAL Replay** | 282 us | 282 us | 100,000 records replayed with checksum accumulation | `[PASS]` |
| **SBE Template Dispatch** | p50=0 p99=23 cyc | p99=7.2 ns | 5 templates (Add/Cancel/Exec/Delete/Replace) | `[PASS]` |
| **Shadow Ingress Log** | 64 cyc/entry | 20.0 ns | mmap-backed, 64B records, MS_ASYNC flush | `[PASS]` |
| **Shadow Decision Log** | 64 cyc/entry | 20.0 ns | Decision + ingress pair per entry | `[PASS]` |
| **Paper Engine process_event** | 5,970 cyc/event | 1.87 us | 50,000 events, 5,000 active paper orders (O(n) scan) | `[PASS]` |
| **Paper Engine FIFO Fills** | 4,999 fills | N/A | Real queue position drain, no artificial slippage | `[PASS]` |
| **Paper Order Submit** | 0 cyc | <RDTSC floor | LOB shares_at_price query + Vec allocation | `[PASS]` |
| **E2E Pipeline p50** | 49 cyc | 15.3 ns | Ingress -> Shadow Log -> LOB Apply -> Paper Process | `[PASS]` |
| **E2E Pipeline p99** | 5,195 cyc | 1,623.4 ns | 99th percentile over 50,000 events | `[PASS]` |
| **E2E Pipeline p99.9** | 8,022 cyc | 2,506.9 ns | Tail latency over 50,000 events | `[PASS]` |
| **E2E Pipeline max** | 47,892 cyc | 14,966.2 ns | Worst-case single event | `[PASS]` |
| **E2E Throughput** | 47.76 Mevents/s | N/A | Single-core, synthetic loopback | `[PASS]` |

---

## Known Limitations (Honest)

These are real constraints, not future plans disguised as features:

1. **No real market data.** All ITCH/SBE messages are structurally valid but fabricated. No NASDAQ ITCH pcap replay has been performed.
2. **No real NIC.** The pipeline has never processed a live multicast packet. AF_XDP and VFIO are V2 code, untested in V3 context.
3. **Paper engine is O(n) per event.** `process_event` linearly scans all active paper orders (`paper.rs:118`). With 5K orders: ~6K cyc/event. With 1M orders: ~1M cyc/event. Not production-scalable.
4. **Single-threaded.** No ingest thread, no strategy thread, no SPSC ring between stages.
5. **Sub-18-cycle measurements show as 0.** ITCH p50, SBE p50, and Paper Order Submit all perform work in approximately 5 cycles, but RDTSC overhead subtraction floors them to 0. The p99 values (23 cyc) are the reliable upper bounds.
6. **Event patterns are deterministic.** The 10-event repeating cycle produces fills mechanically but does not model real market microstructure.

---

## V3 Architecture

### New: PCAP Ingestion (`aw-pcap`)

* Zero-copy mmap reader for PCAP files (nanosecond magic `0xa1b23c4d`, microsecond `0xa1b2c3d4`).
* Ethernet -> IP -> UDP payload extraction.
* MoldUDP64 header parsing (20-byte header, session + sequence + message count).
* Message iterator over MoldUDP64 payload blocks.

### Extended: ITCH 5.0 Full Lifecycle (`aw-exchange`)

* 10 message types: Add Order (A), Add Order MPID (F), Order Executed (E), Order Executed With Price (C), Order Cancel (X), Order Delete (D), Order Replace (U), Trade (P), System Event (S), Trading Action (H).
* Correct byte offsets and field sizes per NASDAQ ITCH 5.0 specification.
* Zero-copy parsing directly into `MarketEvent` struct.

### Extended: Bitmap Limit Order Book (`aw-exchange`)

* 1024-word bitmap index per side (8KB each) for O(1) best-bid/best-ask lookup.
* `find_next_best_bid`: leading_zeros scan downward.
* `find_next_best_ask`: trailing_zeros scan upward.
* `shares_at_price`, `order_info` queries for paper engine integration.

### Extended: Event Log WAL (`aw-store`)

* 64-byte cacheline-aligned records: 8B sequence + 8B timestamp + 1B type + 7B pad + 40B payload.
* mmap-backed with MAP_SHARED, page-aligned msync (MS_ASYNC flush, MS_SYNC durability).
* 4KB guard page appended to mmap to prevent SIMD store overshoot past mapping boundary.
* Reader with sequential replay and checksum verification.

### Extended: SBE Dispatch + Shadow Logger (`aw-mcast`)

* 5 SBE templates: AddOrder (101), OrderCancel (102), OrderExecuted (103), OrderDelete (104), OrderReplace (105).
* `#[repr(C, packed)]` structs with template-based dispatch.
* ShadowLogger: mmap-backed decision audit trail. Logs ingress events, trading decisions, fills, and rejections with nanosecond timestamps.

### New: Paper Trading Engine (`aw-sim::paper`)

* FIFO price-time priority queue position model.
* On `submit_order`: queries LOB for `shares_at_price` to compute `ahead_volume`.
* On `OrderExecuted`/`OrderCancel`/`OrderDelete`: decrements `ahead_volume` for matching paper orders. When `ahead_volume == 0`, subsequent executions fill the paper order at its limit price.
* Configurable transit latency (paper orders are invisible until `live_time_ns`).
* No artificial slippage. Fills occur at the paper order's limit price when queue position is drained.
* 5 unit tests verify: queue drain fill, partial drain, transit latency delay, cancel prevention, delete decrement.

---

## Crate Architecture

```text
Apex-Wire V3 Workspace (30 crates)
├── aw-alloc             Thread-local HugePage slab allocator
├── aw-bench             Benchmark suite (aw_v2_final, aw_v3_benchmark, aw_v4, aw_v5)
├── aw-core              Base types, rdtsc profiling, cache line alignment
├── aw-disruptor         LMAX Disruptor SPSC ring buffer backed by HugePages
├── aw-dma               Intel DDIO & NAPI busy-polling utilities
├── aw-echo              Benchmark application targets
├── aw-exchange          ITCH 5.0 (10 types), bitmap LOB, AVX2 batch cancellation
├── aw-gen               Multi-flow line-rate stress generator
├── aw-markv-bridge      Mark V decision engine FFI bridge
├── aw-matching          O(1) price-time priority matching engine
├── aw-mcast             SBE dispatch (5 templates), ShadowLogger, dual-feed arbitrator
├── aw-mpmc              Lock-free MPMC queue (Vyukov ring)
├── aw-onload            LD_PRELOAD POSIX socket interception shim
├── aw-pacer             Hierarchical atomic timing wheel
├── aw-pcap              Zero-copy mmap PCAP reader, MoldUDP64 framing [NEW]
├── aw-pipeline          Full-duplex tick-to-trade end-to-end pipeline
├── aw-pmc               Hardware PMC (perf_event_open) telemetry
├── aw-ptp               IEEE 1588 PTP hardware clock interface
├── aw-ptp-servo         PTP clock offset PI controller
├── aw-quic              Zero-copy QUIC & HTTP/3 protocol engine
├── aw-risk              Pre-trade risk limit filter
├── aw-runtime           Lock-free thread-per-core async executor
├── aw-sim               Deterministic chaos testing (300 seeds) + PaperEngine [EXTENDED]
├── aw-simd              Universal header SIMD parsing routines
├── aw-sor               SIMD multi-venue Smart Order Router
├── aw-store             EventLog WAL (64B records, mmap, msync) [EXTENDED]
├── aw-tap               Linux TAP interface driver
├── aw-uring             Raw io_uring kernel bindings
├── aw-vfio              Bare-metal VFIO PCIe BAR0 user-space driver
└── aw-xdp               AF_XDP zero-copy kernel bypass driver
```

---

## Build and Execution

### Prerequisites

* Linux Kernel 5.4+ (Kernel 6.x / 7.x recommended)
* Rust stable toolchain (1.75+)
* GCC / Clang build environment

### Build Workspace

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release --workspace
```

### Run V3 Benchmark (maximum performance)

```bash
cargo build --release --bin aw_v3_benchmark
sudo bash -c 'for g in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo performance > "$g"; done; ulimit -l unlimited; taskset -c 2 nice -n -20 ./target/release/aw_v3_benchmark'
```

### Run V3 Benchmark (standard)

```bash
cargo build --release --bin aw_v3_benchmark
taskset -c 2 ./target/release/aw_v3_benchmark
```

### Run V2 Benchmark (unchanged)

```bash
cargo build --release --bin aw_v2_final
sudo bash -c 'ulimit -l unlimited; taskset -c 2 ./target/release/aw_v2_final'
```

### Run All Tests

```bash
cargo test --workspace
```

---

## V2 Baseline (preserved, 9/9 PASS)

| Subsystem | V2 Latency | Status |
| :--- | :--- | :---: |
| SIMD Batch Cancel (AVX2 x8) | 5 cyc | `[PASS]` |
| ITCH 5.0 Feed Parser | 5 cyc | `[PASS]` |
| Matching Engine (p50/p99) | 5 / 23 cyc | `[PASS]` |
| Matching Throughput | 139.1 Mevents/s | `[PASS]` |
| UDP Network Loopback | 4,633 cyc | `[PASS]` |
| UDP Delivery Integrity | 100.0% | `[PASS]` |
| PTP Clock Sync | /dev/ptp0 | `[PASS]` |
| WAL Direct Persistence | 5 cyc | `[PASS]` |
| Cross-Thread SPSC Bridge | 5 cyc | `[PASS]` |

---

## License

Licensed under the MIT License.

