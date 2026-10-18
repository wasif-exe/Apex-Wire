Apex-Wire V1 unifies the complete v1–v10 evolution into a single 29-crate, 23-subsystem tick-to-trade exchange appliance. Written from scratch in Rust and C, it spans kernel-bypass networking (AF_XDP, VFIO PCIe DMA), market data parsing (Nasdaq ITCH 5.0, CME MDP 3.0 SBE, FIX 4.4), a zero-allocation price-time priority matching engine, AVX2 vectorized order management, hardware PTP clock synchronization, a full-duplex E2E pipeline, hardware PMC telemetry, multi-flow stress generation, and a decision engine bridge. All benchmarks measure isolated operations and end-to-end pipeline stages with zero heap allocations, zero syscalls, and zero locks on the measured hot path.

> **Scope disclosure:** Apex-Wire V1 is a from-scratch implementation of a full exchange dataplane in Rust, benchmarked as synthetic microbenchmarks and a synthetic pipeline on a single pinned core. It is **not** a production exchange. The VFIO driver operates in mock mode (no physical NIC attached), the Mark V decision bridge is a stub (EVM mempool domain mismatch with order book matching), the E2E pipeline uses mock network egress, and PTP hardware clock is synthetically emulated. The benchmarks measure isolated hot-path operations and a synthetic end-to-end path, not live order-to-trade latency on a real network.

---

## Architectural Evolution (v1 to V1)

| Version | Target Scope | Core Technical Achievement |
| --- | --- | --- |
| **v1** | Correct 3-Way Handshake | Zero-I/O TCP FSM over Linux TAP, 1MB SHA-256 verified transfers, 300/300 chaos simulation seeds. |
| **v2** | Full L2 to L7 Stack | PAWS-safe timestamps, Window Scaling, SACK negotiation, UDP, DNS stub resolver, HTTP client. |
| **v3** | Kernel Bypass & Transport | AF_XDP zero-copy rings, RFC 6675 SACK Scoreboard, Google BBR congestion control, async `io_uring` reactor, TLS 1.3. |
| **v4** | Hardware-Sympathetic Core | Cycle-accurate profiling (`rdtsc`), flat slab tables, 2MB HugePage UMEM, multi-core RSS sharding, busy-polling. |
| **v5** | Vectorization & Storage | 256-bit AVX2 SIMD parser, eBPF flow steering, `O_DIRECT |
| **v6** | Asynchronous Smart-Node | Zero-Copy QUIC / HTTP/3 QPACK engine, AVX-512 dual-stack parser, `io_uring` SQPOLL NVMe reactor (2.16 GB/s), SmartNIC offloads. |
| **v7** | Sub-Microsecond Dataplane | LMAX Disruptor HugePages (9.02 ns), ITCH 5.0 (19.31 cycles), array LOB (23.38 cycles), eBPF XDP pre-trade risk filter. |
| **v8** | Commercial HFT Appliance | Bare-Metal VFIO PCIe BAR0 DMA (1.31 ns), POSIX shim (`aw-onload`), CME MDP 3.0 SBE (3.87 cycles), Dual-Feed A/B Arbitrator (4.68 cycles), SIMD SOR (0.93 ns), PTP PI Servo. |
| **v9** | Zero-Alloc Matching Engine | Flat-Array Matching (11.88 cyc), HugePage Slab (1.67 ns), Vyukov MPMC (7.63 ns), AVX2 Batch Cancel (1.02 cyc/8). 24 crates, 0 heap allocs, 0 syscalls, 0 locks. |
| **V1** | **Unified Exchange Appliance** | **Full-duplex E2E pipeline (30.0 ns p50), HW PMC telemetry, 1024-flow stress (0% drop), Mark V decision bridge. 29 crates, 23 subsystems, 6 sections.** |

---

## Verified Subsystem Metrics (`aw_v1_benchmark`)

### Hardware & Test Disclosures

* **CPU Host:** x86_64 (AVX2 enabled), pinned to a single core via `taskset -c 2`
* **Compiler:** `rustc` stable, `RUSTFLAGS="-C target-cpu=native"`
* **Optimization:** `[profile.release]` with `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `opt-level = 3`
* **Iteration Count:** 10M–40M iterations per benchmark, 8× manual loop unrolling on matching and SIMD paths
* **Measurement:** `rdtsc` cycle counter and `std::time::Instant` wall-clock with `black_box()` fences to prevent dead-code elimination
* **HugePages:** Slab allocator backed by 1GB HugePages via `aw-disruptor` HugePage infrastructure, with 2MB and 4KB fallback
* **SIMD:** AVX2 intrinsics via `#[target_feature(enable = "avx2")]`, no runtime dispatch overhead on the measured path
* **VFIO:** Mock mode — no physical NIC attached. DMA transmit latency is synthetic.
* **PMC:** `perf_event_open` syscall 298 on x86_64. Hardware counters available on this host.
* **PTP:** No hardware clock detected on `eth0`. W2W latency uses synthetic `rdtsc`-derived timestamps.
* **Mark V Bridge:** STUB mode. Decision kernel spins 185 cycles to simulate ALU work. No live EVM mempool integration.

### Subsystem Performance Results

| # | Subsystem | Type | Throughput | Latency | Status | What Is Actually Measured |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | **Disruptor Ring SPSC** (`aw-disruptor`) | MICRO | 110.87 Mops/s | 9.02 ns/publish | PASS | LMAX-style publish+drain on a 64KB HugePage ring. Single producer, single consumer. |
| 2 | **ITCH 5.0 Vector Parser** (`aw-exchange`) | MICRO | 155.35 Mpps | 19.31 cycles/msg | PASS | Nasdaq ITCH 5.0 AddOrder message decode into `MarketEvent`. No network I/O. |
| 3 | **FIX 4.4 Tag Parser** (`aw-exchange`) | MICRO | 4.32 Mops/s | 694.78 cycles/msg | PASS | ASCII FIX tag-value parsing of a NewOrderSingle message. No TCP framing. |
| 4 | **CME SBE MDP 3.0** (`aw-mcast`) | MICRO | 775.93 Mpps | 3.87 cycles/msg | PASS | Simple Binary Encoding decode of a CME AddOrder message. No multicast socket. |
| 5 | **Dual-Feed A/B Arbitrator** (`aw-mcast`) | MICRO | 640.45 Mops/s | 4.68 cycles/arb | PASS | Sequence-number-based feed arbitration between FeedA and FeedB. No network. |
| 6 | **XDP Pre-Trade Risk** (`aw-risk`) | INTEG | 254.63 Mops/s | 11.78 cycles/chk | PASS | Symbol-level risk limit check (max order size, price bounds, notional). No eBPF in this measurement. |
| 7 | **SIMD Smart Order Router** (`aw-sor`) | MICRO | 1073.56 Mops/s | 0.93 ns/route | PASS | 3-venue BBO comparison and best-venue selection. No network egress. |
| 8 | **Matching Engine** (`aw-matching`) | MATCH | 252.4 Mops/s | 11.88 cyc (4.0 ns) | PASS | Core match loop on a flat-array order book. Single fill per match. No protocol, no persistence. |
| 9 | **Limit Order Book** (`aw-exchange`) | MICRO | 128.30 Mops/s | 23.38 cycles/upd | PASS | Array-backed LOB add-order with price-time priority. 50 price levels. |
| 10 | **SIMD User Cancel ×8** (`aw-exchange`) | SIMD | 23,526 Mslots/s | 1.02 cyc/8 orders | PASS | AVX2 scan of 8 packed order slots comparing user IDs. No memory write-back. |
| 11 | **SIMD Price Sweep ×8** (`aw-exchange`) | SIMD | 26,089 Mslots/s | 0.92 cyc/8 orders | PASS | AVX2 scan of 8 packed order slots comparing price fields. No memory write-back. |
| 12 | **HugePage Slab Allocator** (`aw-alloc`) | MEMORY | 600.3 Mops/s | 1.67 ns/alloc+free | PASS | Thread-local alloc+free of 64-byte blocks from a pre-allocated HugePage slab. |
| 13 | **Lock-Free MPMC Queue** (`aw-mpmc`) | MICRO | 131.0 Mops/s | 7.63 ns/push+pop | PASS | Single-thread push+pop on a Vyukov-style bounded ring. No multi-thread contention. |
| 14 | **io_uring WAL (SQPOLL)** (`aw-store`) | INTEG | 9.84 Mops/s | 101.63 ns/op | PASS | Async key-value append to NVMe WAL via `io_uring` SQPOLL. Real kernel I/O. |
| 15 | **VFIO PCI DMA** (`aw-vfio`) | HW [MOCK] | 765.98 Mops/s | 1.31 ns/tx | PASS | User-space PCI DMA transmit. **Mock mode** — no physical NIC. |
| 16 | **Onload Intercept** (`aw-onload`) | SHIM | 602.81 Mops/s | 4.98 cycles/call | PASS | `LD_PRELOAD` POSIX `send()` interception via FD table lookup. No real socket. |
| 17 | **PTP PI Servo** (`aw-ptp-servo`) | SERVO | 423.59 Mops/s | 7.08 cycles/samp | PASS | PI controller sample for PTP clock offset correction. Synthetic offset input. |
| 18 | **LocalExecutor** (`aw-runtime`) | MICRO | 7.86 Mops/s | 127.20 ns/task | PASS | 200K async task spawn+run on a lock-free thread-per-core executor. |
| 19 | **W2W ITCH→Risk→LOB** (`aw-ptp`) | SYNTH | 1,000 kops/s | p50:13ns p99:19ns | PASS | Full decode→risk→book pipeline with `rdtsc` latency histogram. Synthetic timestamps. |
| 20 | **E2E Tick-to-Trade** (`aw-pipeline`) | SYNTH | 33.4 Mops/s | 30.0 ns p50 | PASS | Disruptor→Risk→Match→Decision→SOR→VFIO pipeline. **Synthetic** — mock NIC, no real network. |
| 21 | **HW PMC Telemetry** (`aw-pmc`) | PMC | PERF_AVAILABLE | snapshot captured | PASS | `perf_event_open` hardware counters: cycles, instructions, L1D/LLC misses, branch misses. |
| 22 | **Multi-Flow Stress 1024f** (`aw-gen`) | STRESS | 102,400 events | p99.99 10561ns d0.0000 | PASS | 1024 concurrent flows through the full pipeline. Zero drops. |
| 23 | **Mark V Decision Bridge** (`aw-markv`) | STUB | 61.6 ns/dec | STUB mode | PASS | Decision kernel bridge. **STUB** — 185-cycle spin simulating ALU work. No live EVM integration. |

### W2W Latency Percentile Distribution (ITCH Decode → Risk → LOB)

| Min | p50 | p90 | p99 | p99.9 | p99.99 | Max | Mean |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 10 ns | 13 ns | 15 ns | 19 ns | 21 ns | 99 ns | 2,268,154 ns | 36.71 ns |

### Benchmarking Methodology

* **What we count:** Only successfully completed operations. Rejected, failed, or no-op iterations are excluded from throughput calculations.
* **What we do not count:** Setup time, memory pre-allocation, HugePage mapping, or any one-time initialization cost. Benchmarks measure steady-state hot-path performance only.
* **`black_box` usage:** All benchmark loops pass results through `std::hint::black_box()` to prevent the compiler from optimizing away the measured work. This adds a small but consistent overhead to every iteration.
* **Loop unrolling:** 8× manual unrolling is applied to matching engine and SIMD benchmarks to reduce loop branch overhead. This is a standard microbenchmark technique and does not reflect typical production call patterns.
* **Single-core pinning:** `taskset -c 2` eliminates cross-core migration and cache cold-start effects. Real workloads will see higher latency due to core switching and cache contention.
* **Honest labeling:** Every subsystem is labeled with its actual measurement type: `MICRO` (isolated operation), `INTEG` (kernel I/O involved), `HW` (hardware path), `SYNTH` (synthetic pipeline), `STUB` (simulated external system), `PMC` (hardware counter probe), `STRESS` (multi-flow load test). No synthetic result is labeled as hardware-proven.

### Context: Production System Comparison

> **Important caveat:** The numbers below compare our microbenchmark hot-path operations against production systems that include full protocol stacks, journaling, risk management, and fault tolerance. **These are not equivalent measurements.** The purpose is to show that the core algorithms are architecturally sound, not to claim parity with production exchanges.

| System | Reported Latency | Scope |
| --- | --- | --- |
| **Apex-Wire V1 Matching** (this project) | ~12 cycles (~4.0 ns) | Core match loop only, flat array, no protocol, no persistence |
| **Apex-Wire V1 E2E Pipeline** (this project) | ~30 ns p50 | Full synthetic pipeline: decode→risk→match→decision→route→egress. Mock NIC. |
| **LMAX Disruptor** (public estimates) | ~50–200 cycles | Full matching engine with journaling and replication |
| **NASDAQ INET** (public estimates) | ~15,000 cycles (~5 µs) | Full exchange stack including risk, routing, and market data |
| **Apex-Wire V1 Slab Alloc** (this project) | ~1.67 ns | Thread-local, 3 fixed size classes (64/128/256B), no fragmentation |
| **glibc `malloc**` (typical) | ~25–80 ns | Arbitrary sizes, thread-safe, fragmentation-aware |

---

## Subsystem Deep Dive (Apex-Wire V1 Architecture)

### Section 1: Network Ingress (`aw-disruptor`, `aw-exchange`, `aw-mcast`)

* **LMAX Disruptor Ring:** Lock-free SPSC ring buffer backed by HugePages (1GB → 2MB → 4KB fallback). Publisher claims a sequence number, writes directly into the slot, and publishes. Consumer drains in batches. The 9.02 ns measurement includes claim + write + publish + periodic drain.
* **ITCH 5.0 Parser:** Decodes Nasdaq ITCH 5.0 AddOrder messages (message type `A`) into a 64-byte aligned `MarketEvent` struct. Field extraction uses big-endian byte slicing with no heap allocation.
* **FIX 4.4 Parser:** ASCII tag-value parser for FIX 4.4 NewOrderSingle messages. Scans for `|` delimiters, extracts tag numbers and values into a fixed-size `FixMessage` struct.
* **CME SBE Decoder:** Simple Binary Encoding decoder for CME MDP 3.0 market data. Parses an 8-byte `SbeHeader` followed by a 24-byte `SbeAddOrder` payload. Little-endian field extraction.
* **Dual-Feed Arbitrator:** Sequence-number-based A/B feed arbitrator. Tracks the highest seen sequence per feed source and suppresses duplicates. Reset clears all state.

### Section 2: Risk & Routing (`aw-risk`, `aw-sor`)

* **Pre-Trade Risk Filter:** Per-symbol risk limits stored in a flat array indexed by 8-byte symbol key. Checks max order size, min/max price, and max notional in a single pass. Returns a `RiskVerdict` enum (Pass, RejectMaxOrderSize, RejectPriceBounds, RejectNotional).
* **SIMD Smart Order Router:** 3-venue BBO comparison using scalar min/max. `update_venue_bbo()` maintains best bid/ask per venue. `route_order()` selects the venue with the best price for the given side and quantity.

### Section 3: Matching Core (`aw-matching`, `aw-exchange`)

* **Flat-Array Order Book:** Fixed-size arrays (`[PriceLevel; 65536]` for bids and asks, `[OrderSlot; 65536]` for orders). All data pre-allocated at startup — zero heap allocations during matching.
* **O(1) Price Lookup:** Price levels indexed directly by price value. Best bid/ask tracking uses bounded linear scans with `min_bid`/`max_ask` watermarks.
* **Order Slot Layout:** Each `OrderSlot` is exactly 16 bytes (`align(16)`): `user_id: u32`, `order_id: u32`, `price: u32`, `qty: u16`, `side: u8`, `active: u8`. 4 slots per 64-byte cache line.
* **Match Loop:** Incoming market orders walk the opposing book from best price, filling against resting limit orders in price-time priority. Each fill produces a `MatchResult` with fill count and quantity.
* **AVX2 Batch Cancel:** `PackedOrderSlots` wraps a 32-byte-aligned order slot array. `batch_cancel_by_user_x8` loads 8 user IDs into a `__m256i` register and compares in one instruction. `batch_cancel_by_price_below_x8` does the same for price fields.

### Section 4: Memory & Concurrency (`aw-alloc`, `aw-mpmc`)

* **HugePage Slab Allocator:** Three fixed size classes (64, 128, 256 bytes). Each backed by a contiguous HugePage allocation. Intrusive free list threaded through the first 8 bytes of each free block. Thread-local — no atomics, no locks, no cross-thread free. `SlabBox<T>` provides RAII allocation.
* **Lock-Free MPMC Queue:** Dmitry Vyukov bounded ring algorithm. Per-slot sequence counters with `compare_exchange`. Producer and consumer cursors `align(64)` to prevent false sharing. `cpu_relax()` (PAUSE) spin on contention.

### Section 5: Infrastructure (`aw-store`, `aw-vfio`, `aw-onload`, `aw-ptp-servo`, `aw-runtime`)

* **io_uring WAL:** Async write-ahead log using `io_uring` with SQPOLL mode (kernel-side submission queue polling). `append_set()` submits key-value writes. Real kernel I/O path — the only benchmark that touches the kernel on the measured path.
* **VFIO PCI DMA:** User-space PCI driver using VFIO `ioctl` for BAR0 mapping and DMA transmit. **Mock mode** in current benchmarks — `VfioDevice::open_mock()` simulates the DMA path without a physical NIC. Real mode requires `vfio-pci` kernel driver and IOMMU group binding.
* **Onload Intercept:** `LD_PRELOAD` shim that intercepts POSIX `send()`/`recv()` calls. FD table maps file descriptors to `FdType` (WireIntercepted vs Passthrough). Intercepted calls route through the Apex-Wire dataplane.
* **PTP PI Servo:** Proportional-Integral controller for IEEE 1588 PTP clock synchronization. `sample_ptp_offset()` takes a measured offset and returns a frequency adjustment. Maintains `phase_error_ps` state.
* **LocalExecutor:** Lock-free thread-per-core async runtime. `spawn()` enqueues a future into a task ring. `run_until_stalled()` drains the ring. No cross-core work stealing.

### Section 6: Full Pipeline (`aw-pipeline`, `aw-pmc`, `aw-gen`, `aw-markv-bridge`, `aw-ptp`)

* **E2E Tick-to-Trade Pipeline:** `Pipeline::new()` wires Disruptor → Risk → Matching → Decision → SOR → VFIO into a single `process_event()` call. `PipelineResult` is `#[repr(C, align(64))]` (64 bytes, one cache line). Mock Tx completion reaping prevents ring overflow in mock mode.
* **HW PMC Telemetry:** `PmcProfiler` opens `perf_event_open` syscall 298 for 5 hardware counters: CPU cycles, instructions retired, L1D cache misses, LLC misses, branch misses. `PmcSnapshot` provides IPC, L1D miss rate, and LLC miss rate. Graceful fallback to `PmcStatus::Fallback` if counters are unavailable.
* **Multi-Flow Stress Generator:** `StressGenerator::new(N)` creates N concurrent flows with unique symbols. `run(events_per_flow)` pushes all events through the pipeline and measures per-flow latency percentiles and drop rate. PASS requires 0% drop.
* **Mark V Decision Bridge:** FFI bridge to the Mark V decision engine. **STUB mode** — `MarkVBridge::new_stub()` spins 185 cycles to simulate decision ALU work. `BridgeMode::Live` would mount a real Mark V kernel via zero-copy shared memory, but the Mark V project (`rust_soldier`) targets EVM mempool sniping (async tokio, `alloy::Address`), which is fundamentally incompatible with the synchronous zero-syscall order book hot path.
* **W2W Latency Histogram:** `LatencyHistogram` records per-event `rdtsc` deltas across the ITCH→Risk→LOB path. `compute_percentiles()` returns min/p50/p90/p99/p99.9/p99.99/max/mean.

---

## Workspace Package Structure (29 Crates)

```text
.
├── Cargo.toml                              # Workspace manifest (29 members)
├── aw-core                                 # Base types, probes, rdtsc, profiling, sharding
│   └── src
│       ├── lib.rs                          # Re-exports
│       ├── types.rs                        # MarketEvent, Symbol, Price, Qty
│       ├── profile.rs                      # StageId (27 stages), CycleProfiler
│       ├── cacheline.rs                    # CacheLine<T> align(64) wrapper
│       ├── conntable.rs                    # Connection table
│       ├── kv.rs                           # Key-value store
│       ├── resp.rs                         # Response types
│       └── shard.rs                        # Multi-core RSS sharding
├── aw-exchange                             # ITCH 5.0, FIX 4.4, LOB, SIMD Cancel
│   └── src
│       ├── lib.rs                          # Re-exports
│       ├── order.rs                        # MarketEvent, EventKind, Side
│       ├── lob.rs                          # LimitOrderBook (array-backed)
│       ├── itch.rs                         # parse_itch_message()
│       ├── fix.rs                          # parse_fix(), FixMessage
│       └── simd_cancel.rs                  # AVX2 batch cancel kernels
├── aw-matching                             # [v9] Price-Time Priority Matching Engine
│   └── src
│       └── lib.rs                          # MatchingEngine, MatchResult, OrderSlot, PriceLevel
├── aw-alloc                                # [v9] Thread-Local HugePage Slab Allocator
│   └── src
│       └── lib.rs                          # HugeSlabAllocator, FixedSlab, SlabBox
├── aw-mpmc                                 # [v9] Lock-Free MPMC Queue
│   └── src
│       └── lib.rs                          # MpmcQueue (Vyukov bounded ring)
├── aw-disruptor                            # LMAX Disruptor HugePage Ring Buffer
│   └── src
│       ├── lib.rs                          # Disruptor<T>, Publisher, Consumer
│       └── hugepage.rs                     # HugePage allocation (1GB/2MB/4KB)
├── aw-mcast                                # CME MDP 3.0 SBE Decoder & Feed Arbitrator
│   └── src
│       ├── lib.rs                          # Re-exports
│       ├── sbe.rs                          # SbeHeader, SbeAddOrder, parse_sbe_message()
│       └── arbitrator.rs                   # FeedArbitrator, FeedSource
├── aw-vfio                                 # Bare-Metal VFIO PCIe BAR0 User-Space Driver
│   └── src
│       ├── lib.rs                          # Re-exports
│       ├── device.rs                       # VfioDevice (real + mock)
│       ├── ring.rs                         # VfioTxRing, VfioRxRing
│       └── vfio_sys.rs                     # VFIO ioctl bindings
├── aw-onload                               # LD_PRELOAD POSIX Interception Shim
│   └── src
│       ├── lib.rs                          # Re-exports
│       ├── shim.rs                         # aw_send(), aw_recv(), get_fd_table()
│       └── fd_table.rs                     # FdType, FdTable
├── aw-sor                                  # SIMD Multi-Venue Smart Order Router
│   └── src
│       ├── lib.rs                          # Re-exports
│       └── router.rs                       # SmartOrderRouter
├── aw-ptp-servo                            # PTP Hardware Clock PI Servo Controller
│   └── src
│       ├── lib.rs                          # Re-exports
│       └── servo.rs                        # PiServoController, PiServoConfig
├── aw-pipeline                             # [V1] Full-Duplex Tick-to-Trade Pipeline
│   └── src
│       └── lib.rs                          # Pipeline, PipelineResult
├── aw-pmc                                  # [V1] Hardware PMC Telemetry Profiler
│   └── src
│       └── lib.rs                          # PmcProfiler, PmcSnapshot, PmcStatus
├── aw-gen                                  # [V1] Multi-Flow Line-Rate Stress Generator
│   └── src
│       └── lib.rs                          # StressGenerator, StressReport
├── aw-markv-bridge                         # [V1] Mark V Decision Engine Bridge
│   └── src
│       └── lib.rs                          # MarkVBridge, MarkVDecision, BridgeMode
├── aw-dma                                  # Intel DDIO & NAPI Busy-Polling
│   └── src
│       ├── lib.rs
│       ├── ddio.rs
│       └── napi.rs
├── aw-risk                                 # Pre-Trade eBPF XDP Risk Filter
│   ├── bpf
│   │   └── xdp_risk_filter.c
│   └── src
│       ├── lib.rs
│       └── filter.rs                       # PreTradeRiskFilter, SymbolRiskLimits
├── aw-ptp                                  # IEEE 1588 PTP Hardware Clock Reader
│   └── src
│       ├── lib.rs
│       ├── hw_timestamp.rs
│       └── latency.rs                      # LatencyHistogram, LatencySummary
├── aw-quic                                 # Zero-Copy QUIC & HTTP/3 Engine
│   └── src
│       ├── lib.rs
│       ├── connection.rs
│       ├── crypto.rs
│       ├── frame.rs
│       ├── h3.rs
│       ├── packet.rs
│       ├── stream.rs
│       └── varint.rs
├── aw-simd                                 # Universal Network Header SIMD Parser
│   ├── benches
│   │   └── parse.rs
│   └── src
│       ├── lib.rs
│       ├── avx2.rs
│       ├── avx512.rs
│       ├── batch.rs
│       ├── masks.rs
│       └── scalar.rs
├── aw-runtime                              # Lock-Free Thread-per-Core Async Runtime
│   └── src
│       ├── lib.rs
│       ├── executor.rs                     # LocalExecutor
│       ├── task.rs
│       └── waker.rs
├── aw-store                                # io_uring SQPOLL Direct I/O WAL Engine
│   └── src
│       ├── lib.rs
│       ├── crc.rs
│       ├── wal.rs
│       └── uring_wal.rs                    # AsyncWalWriter
├── aw-xdp                                  # AF_XDP Zero-Copy Kernel Bypass Engine
│   ├── bpf
│   │   ├── xdp_prog.c
│   │   └── xdp_prog.o
│   └── src
│       ├── lib.rs
│       └── hints.rs
├── aw-pacer                                # Hierarchical Atomic Timing Wheel
│   └── src
│       ├── lib.rs
│       └── wheel.rs
├── aw-sim                                  # Deterministic Chaos Simulator (300 seeds)
│   └── src
│       ├── lib.rs
│       └── main.rs
├── aw-tap                                  # Linux TAP Interface Driver
│   └── src
│       ├── lib.rs
│       └── bin
│           └── probe.rs
├── aw-uring                                # io_uring Low-Level Bindings
│   └── src
│       └── lib.rs
├── aw-echo                                 # Benchmark Suite and Applications
│   └── src
│       ├── main.rs
│       └── bin
│           └── aw_v1_benchmark.rs          # [V1] Unified 23-Subsystem Dashboard
└── scripts
    ├── clean_all_comments.py
    ├── tap-down.sh
    ├── tap-up.sh
    ├── xdp-down.sh
    ├── xdp-up-multiqueue.sh
    └── xdp-up.sh

```

### Hardware Flags

| Flag | Status | Notes |
| --- | --- | --- |
| VFIO | MOCK | No physical NIC attached. `VfioDevice::open_mock()` simulates DMA. |
| SQPOLL | ACTIVE | `io_uring` kernel-side submission queue polling is available. |
| PMC | PERF_AVAILABLE | `perf_event_open` hardware counters functional on this host. |
| PTP | SYNTH | No hardware PTP clock on `eth0`. Timestamps are `rdtsc`-derived. |
| Pipeline | SYNTHETIC | E2E pipeline uses mock NIC egress. No real network I/O. |
| Mark V | STUB | EVM mempool domain mismatch. 185-cycle spin simulates decision work. |

### Backward Compatibility

Apex-Wire V1 adds 5 new crates (`aw-pipeline`, `aw-pmc`, `aw-gen`, `aw-markv-bridge`, and the unified benchmark) and extends `aw-core`'s `StageId` enum from 21 to 27 variants. All 24 existing v1–v9 crates are API-compatible. Specifically:

* `aw-sim` deterministic chaos simulator passes all 300 seeds unchanged
* All public APIs in `aw-exchange`, `aw-disruptor`, `aw-core`, `aw-simd`, `aw-matching`, `aw-alloc`, and `aw-mpmc` are preserved
* The new `simd_cancel` module in `aw-exchange` is additive (new `pub mod`, no changes to existing types)
* `MatchResult::default()` was added to `aw-matching` as a non-breaking additive impl
* The `wire-*` → `aw-*` namespace migration is a clean rename with no API changes

---

## Quickstart and Execution

### Build Workspace

```bash
cargo build --all --release

```

### Run Full Test Suite

```bash
cargo test --all --release

```

### Execute Apex-Wire V1 Unified Benchmark Dashboard

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release --bin aw_v1_benchmark
taskset -c 2 target/release/aw_v1_benchmark

```

---

## License

Licensed under the MIT License.
