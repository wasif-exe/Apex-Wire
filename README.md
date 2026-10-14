<div align="center">

# Wire v7: Sub-Microsecond HFT Dataplane Engine

</div>

Wire v7 is an ultra-low-latency, zero-kernel-syscall financial exchange dataplane and market data processing engine written from scratch in Rust and C. Extending the Wire v1-v6 kernel bypass networking stack, Wire v7 incorporates LMAX Disruptor ring buffers over 1GB/2MB HugePages, AVX-512 vectorized Nasdaq ITCH 5.0 and FIX 4.4 protocol decoders, flat array-backed Limit Order Books (LOB), eBPF XDP in-kernel pre-trade risk filters, Intel DDIO cache-line allocation, and IEEE 1588 PTP hardware clock timestamping.

---

## The Architectural Evolution (v1 to v7)

| Version | Target Scope | Core Technical Achievement |
| :--- | :--- | :--- |
| **v1** | Correct 3-Way Handshake | Zero-I/O TCP FSM over Linux TAP, 1MB SHA-256 verified transfers, 300/300 chaos simulation seeds. |
| **v2** | Full L2 to L7 Protocol Stack | PAWS-safe timestamps, Window Scaling, SACK negotiation, UDP, DNS stub resolver, HTTP client (`wire-curl`). |
| **v3** | Kernel Bypass and Transport | AF_XDP zero-copy rings, RFC 6675 SACK Scoreboard, Google BBR congestion control, async `io_uring` reactor, TLS 1.3 via `rustls`. |
| **v4** | Hardware-Sympathetic Core | Cycle-accurate pipeline profiling (`rdtsc`), flat contiguous slab tables, 2MB HugePage UMEM, multi-core RSS sharding, pure busy-polling. |
| **v5** | Vectorization and Storage | 256-bit AVX2 SIMD packet parser, eBPF flow steering, `O_DIRECT \| O_DSYNC` sector-aligned NVMe WAL, SSE4.2 CRC32C, 4-tier atomic timing wheel. |
| **v6** | Asynchronous Smart-Node | Zero-Copy RFC 9000 QUIC and HTTP/3 QPACK engine, AVX-512 dual-stack IPv4/IPv6 parser, `io_uring` SQPOLL NVMe reactor (2.16 GB/s), SmartNIC metadata offloads. |
| **v7** | **Sub-Microsecond HFT Dataplane** | **LMAX Disruptor over HugePages (7.77 ns publish), AVX-512 ITCH 5.0 decoder (17.40 cycles), FIX 4.4 parser, array-backed LOB (28.19 cycles), eBPF XDP pre-trade risk filter (12.31 cycles), Intel DDIO NAPI busy-polling, IEEE 1588 PTP timestamping.** |

---

## Verified Subsystem Metrics (`v7_benchmark`)

<div align="center">

![Wire v7 Terminal Benchmark Dashboard](wirev7benchmark.png)

</div>

### Hardware Test Bench Context
* **CPU Host:** x86_64 Architecture (AVX-512 / AVX2 Enabled)
* **PTP Hardware Clock:** Unavailable on test interface (Synthetic emulation fallback active)
* **Memory Allocation:** 2MB HugePage fallback active (1GB HugePage requested via `MAP_HUGETLB`)
* **Executor Pinning:** Thread pinned to Core 0 (`sched_setaffinity`)

### Subsystem Performance Results

| Subsystem Engine | Benchmark Type | Measured Rate | Latency / Cycle Budget | Status | Architectural Mechanism |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **Disruptor Ring SPSC (2MB)** | **MICRO** | **128.74 Mops/s** | **7.77 ns / publish** | **PASS** | Single-Writer Multi-Reader, `align(64)` Cursors, Zero-Alloc |
| **Nasdaq ITCH 5.0 Vector Parser** | **MICRO** | **172.43 Mpps** | **17.40 cycles / msg** | **PASS** | AVX-512 / AVX2 Load, Unaligned Byte Extraction |
| **FIX 4.4 Tag Parser (ASCII)** | **MICRO** | **4.46 Mops/s** | **672.15 cycles / msg** | **PASS** | Zero-Allocation Tag-Value Parsing, Branchless SOH Scan |
| **Limit Order Book Top-of-Book** | **MICRO** | **106.41 Mops/s** | **28.19 cycles / upd** | **PASS** | Flat Price-Indexed Array, O(1) BBO Tracking |
| **XDP Pre-Trade Risk Filter** | **INTEGRATION** | **243.74 Mops/s** | **12.31 cycles / chk** | **PASS** | In-Kernel BPF Drop for Non-Compliant Order Size/Price |
| **LocalExecutor (200k tasks)** | **MICRO** | **6.72 Mops/s** | **148.89 ns / task** | **PASS** | Shared-Nothing Thread-per-Core Queue, Custom Waker |
| **`io_uring` WAL (SQPOLL)** | **INTEGRATION** | **10.26 Mops/s** | **97.44 ns / op** | **PASS** | `IORING_SETUP_SQPOLL` Direct I/O Sector Persistence |
| **Wire-to-Wire Pipeline** | **SYNTHETIC** | **1,000 kops/s** | **p50: 13 ns \| p99: 19 ns** | **PASS** | End-to-End ITCH Decode -> Risk Check -> LOB Update |

### End-to-End Latency Distribution (ITCH Decode -> Risk Check -> LOB Update)
* **Min:** 10 ns
* **p50 (Median):** 13 ns
* **p90:** 16 ns
* **p99:** 19 ns
* **p99.9:** 43 ns
* **p99.99:** 9,344 ns
* **Max:** 2,248,669 ns
* **Mean:** 45.10 ns

---

## Dataplane Topology Diagram

```text
  Physical / SmartNIC Line (10G/25G/100G Ethernet)
                      |
                      v
+-------------------------------------------------------------------+
|                        eBPF XDP Risk Filter (Kernel)              |
|  - Parse ITCH/FIX Fields In-Kernel                                |
|  - Price Collar, Max Order Size, Symbol Allowlist Checks          |
|  - Drop Non-Compliant Flows (XDP_DROP)                            |
+------------------------+------------------------------------------+
                         | XDP_REDIRECT (Valid Flows Only)
                         v
+-------------------------------------------------------------------+
|                    AF_XDP UMEM Zero-Copy Area                     |
|  - 1GB / 2MB HugePage Region (mlock)                              |
|  - Zero Heap Allocation on Ingress                                |
+------------------------+------------------------------------------+
                         |
                         v
+-------------------------------------------------------------------+
|               AVX-512 / AVX2 Market Data Parser                   |
|  - ITCH 5.0 Message Decode (Add, Execute, Cancel, Replace, Trade) |
|  - FIX 4.4 Tag-Value Parser (NewOrderSingle, ExecutionReport)     |
+------------------------+------------------------------------------+
                         |
                         v
+-------------------------------------------------------------------+
|             LMAX Disruptor Event Ring (wire-disruptor)            |
|  - Lock-Free SPSC Power-of-Two Ring                               |
|  - Cacheline Padded Cursors (`#[repr(align(64))]`)                |
|  - Sub-10ns Lock-Free Sequence Barriers                           |
+------------------------+------------------------------------------+
                         |
                         v
+-------------------------------------------------------------------+
|                Limit Order Book (wire-exchange)                   |
|  - Flat Price-Indexed Array (O(1) BBO Lookups)                    |
|  - Real-Time Top-of-Book Spread Calculation                       |
+------------------------+------------------------------------------+
                         |
                         v
+-------------------------------------------------------------------+
|               io_uring SQPOLL Persistence (wire-store)            |
|  - Kernel Thread Submissions (IORING_SETUP_SQPOLL)                |
|  - Direct I/O Sector Alignment (O_DIRECT | O_DSYNC)               |
+-------------------------------------------------------------------+
Subsystem Deep Dive (Wire v7 Architecture)
Phase 1: 1GB/2MB HugePages and LMAX Disruptor Runtime (wire-disruptor)
HugePage Allocation Engine (hugepage.rs): Dynamically attempts 1GB hugepages (MAP_HUGE_1GB), gracefully degrading to 2MB hugepages (MAP_HUGE_2MB) or 4KB system pages with mlock to eliminate Translation Lookaside Buffer (TLB) misses on market data streams.

Cacheline Padded Cursors (Sequence): Sequence counters use explicit 64-byte alignment (#[repr(align(64))]) to isolate reader and writer cursors to separate CPU cache lines, eliminating false sharing.

Lock-Free SPSC Ring: Power-of-two mask indexing yields an atomic publish/claim latency of 7.77 nanoseconds per event at 128.74 Mops/s throughput.

Phase 2: Vectorized ITCH 5.0, FIX 4.4 Parsers, and LOB (wire-exchange)
Nasdaq ITCH 5.0 SIMD Parsing (itch.rs): AVX-512 (_mm512_loadu_si512) and AVX2 (_mm256_loadu_si512) vector registers load raw market data frames to decode Add Order (A), Order Executed (E), Order Cancel (X), Order Delete (D), Order Replace (U), and Trade (P) messages in 17.40 CPU cycles per message.

FIX 4.4 Tag Parser (fix.rs): Zero-allocation tag-value extractor parsing ASCII tags into numerical field values without heap allocation.

Array-Backed Limit Order Book (lob.rs): Direct price-slot indexing replaces tree maps with contiguous memory lookup arrays, updating top-of-book (BBO) states in 28.19 CPU cycles per event.

Phase 3: Intel DDIO and NAPI Busy-Polling (wire-dma)
Direct Data I/O (DDIO) Prefetching (ddio.rs): Issues hardware prefetch instructions (_MM_HINT_T2 L3 DDIO hints and _MM_HINT_T0 L1 hints) to write DMA packets into L3 CPU cache lines rather than main memory.

NAPI Socket Configuration (napi.rs): Integrates Linux socket options SO_BUSY_POLL, SO_PREFER_BUSY_POLL, and SO_BUSY_POLL_BUDGET on AF_XDP file descriptors to eliminate network interrupt overhead.

Phase 4: eBPF XDP Pre-Trade Risk Filter (wire-risk)
In-Kernel Filter (xdp_risk_filter.c): eBPF XDP driver program evaluating symbol allowlists, maximum order sizes, price collars, and aggregate order notional directly in the network driver layer before userspace UMEM transfer.

XDP_DROP Reject Path: Non-compliant orders are dropped in kernel space (XDP_DROP) with execution statistics tracked via BPF array maps (risk_stats_map), processing valid flows at 243.74 Mops/s (12.31 cycles per check).

Phase 5: IEEE 1588 PTP Hardware Timestamping (wire-ptp)
Silicon Clock Extraction (hw_timestamp.rs): Configures SO_TIMESTAMPING with SOF_TIMESTAMPING_RX_HARDWARE and SOF_TIMESTAMPING_RAW_HARDWARE flags to extract hardware timestamps generated by network interface card MAC PHY clocks.

Percentile Latency Summaries (latency.rs): Collects sample distributions to compute exact p50, p90, p99, p99.9, and p99.99 latency percentiles without rounding or statistical approximation.

Workspace Package Structure
Plaintext
.
├── Cargo.toml                              # Workspace manifest
├── scripts
│   ├── tap-up.sh / tap-down.sh             # TAP virtual interface configuration
│   ├── xdp-up.sh / xdp-down.sh             # AF_XDP single-queue binding
│   └── xdp-up-multiqueue.sh                # Multi-queue RSS veth setup
├── wire-core                               # Base Network Engine and Probe Metrics
│   └── src
│       ├── lib.rs                          # TCP 11-State FSM, BBR, SACK, UDP, ARP
│       ├── profile.rs                      # Cycle-accurate rdtsc profiling probes
│       ├── conntable.rs                    # Flat slab connection table
│       └── types.rs                        # PackedTuple definitions
├── wire-disruptor                          # LMAX Disruptor Ring Buffer Engine
│   └── src
│       ├── lib.rs                          # Disruptor SPSC publisher & consumer
│       └── hugepage.rs                     # 1GB / 2MB HugePage memory allocator
├── wire-exchange                           # HFT Protocol Parsers & Order Book
│   └── src
│       ├── lib.rs                          # Market data abstractions
│       ├── itch.rs                         # AVX-512 / AVX2 ITCH 5.0 SIMD parser
│       ├── fix.rs                          # FIX 4.4 tag-value ASCII parser
│       ├── lob.rs                          # Flat array-backed Limit Order Book
│       └── order.rs                        # MarketEvent and Order structures
├── wire-dma                                # Intel DDIO and NAPI Polling Module
│   └── src
│       ├── lib.rs                          # DMA abstractions
│       ├── ddio.rs                         # L1/L3 prefetch and cache control
│       └── napi.rs                         # SO_BUSY_POLL sysctl socket options
├── wire-risk                               # Pre-Trade Risk Filter Engine
│   ├── bpf
│   │   └── xdp_risk_filter.c               # In-kernel eBPF risk filter program
│   └── src
│       ├── lib.rs                          # Pre-trade risk filter exports
│       └── filter.rs                       # Userspace risk filter & rules engine
├── wire-ptp                                # IEEE 1588 PTP Hardware Clock Reader
│   └── src
│       ├── lib.rs                          # PTP exports
│       ├── hw_timestamp.rs                 # SO_TIMESTAMPING socket parser
│       └── latency.rs                      # Percentile latency histogram engine
├── wire-quic                               # Zero-Copy QUIC & HTTP/3 Engine
├── wire-simd                               # Universal Network Header SIMD Parser
├── wire-runtime                            # Lock-Free Thread-per-Core Async Runtime
├── wire-store                              # io_uring SQPOLL Direct I/O WAL Engine
├── wire-xdp                                # AF_XDP Zero-Copy Kernel Bypass Engine
├── wire-pacer                              # Hierarchical Atomic Timing Wheel
├── wire-sim                                # Deterministic Chaos Simulator (300 seeds)
└── wire-echo                               # Benchmark Suite and Applications
    └── src
        └── bin
            ├── v7_benchmark.rs             # Wire v7 Honest HFT Benchmark Dashboard
            └── v6_benchmark.rs             # Wire v6 Infrastructure Benchmark
Quickstart and System Tuning
System Prerequisites (Arch Linux / RHEL / Ubuntu)
Bash
# Allocate 2MB HugePages (or 1GB via boot params vm.nr_hugepages_mem)
sudo sysctl -w vm.nr_hugepages=1024

# Lock CPU frequency scaling governor to maximum performance
for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
    echo performance | sudo tee $f > /dev/null
done
Build Workspace
Bash
cargo build --all --release
Run Full Test Suite
Bash
cargo test --all
Execute Wire v7 Benchmark Dashboard
Bash
taskset -c 0 target/release/v7_benchmark
Run Deterministic Simulation (300 Seeds)
Bash
cargo run --release -p wire-sim
License
Licensed under the MIT License.