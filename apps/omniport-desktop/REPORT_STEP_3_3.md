# OmniPort Step 3.3 Verification Report: Visual Topology Canvas & 60 Hz Telemetry Strip Chart

**Date:** 2026-11-10  
**Phase:** 3 — Tauri v2 + React 19 Workbench GUI  
**Step:** 3.3 — Build the `@xyflow/react` Visual Topology Canvas and `uPlot` 60 Hz real-time telemetry strip chart [FR-202, FR-301, FR-302, NFR-104]  
**Status:** PASSED (Verification Complete)

---

## 1. Architectural Context & Invariants

To monitor complex HIL test rigs and multi-drop serial networks, OmniPort requires:
1. **Interactive Routing Matrix Visualization (FR-202):** A node-based routing graph displaying hardware UART endpoints, virtual pairs, $1:N$ splitters, $N:1$ atomic mergers, and fault injectors.
2. **60 Hz Real-Time Telemetry Strip Chart (NFR-104):** Continuous, smooth strip charting of packet throughput, frame rates, Gaussian jitter, and CRC error bursts with zero garbage collection spikes.
3. **Rule `tauri-react-ui.md` Compliance:** High-frequency data paths must bypass React component state (`useState`/`zustand` arrays of packets), using dedicated typed-array ring buffers and direct canvas/WebGL rendering.

Step 3.3 integrates `@xyflow/react` for visual topology orchestration and `uPlot` with a zero-allocation `Float64Array` ring buffer for 60 Hz telemetry plotting.

---

## 2. Component Implementation Details

### 2.1 Visual Topology Canvas (`apps/omniport-desktop/src/components/Topology`)
- **Node Components:**
  - `EndpointNode`: Displays hardware/virtual port path (`COM1`, `/dev/pts/1`), baud rate and framing (`115200 8N1`), connection status indicator, TX/RX byte statistics, and error counters.
  - `SplitterNode`: Represents $1:N$ zero-copy packet fanout with configurable drop policy (`drop_oldest` / `drop_newest`) and queue capacity.
  - `MergerNode`: Represents $N:1$ lock-free atomic frame merger enforcing atomic frame serialization (FR-203).
  - `FaultNode`: Inline edge fault injector displaying fixed delay (ms), Box-Muller Gaussian jitter ($\mu \pm \sigma$), bit error rate (BER), and CRC corruption periodicity.
- **Edge Component (`AnimatedRouteEdge`):**
  - Animated SVG stroke dashes reflecting live packet flow velocity.
  - Center label pill rendering real-time packet rate (e.g. `140 pkt/s`) and error alerts.
- **Canvas Container (`TopologyCanvas.tsx`):**
  - Built with `@xyflow/react` v12, `MiniMap`, `Controls`, background dot grid, and interactive node addition toolbar (`+ Add Port Endpoint`, `+ Add Fault Injector`).

### 2.2 Telemetry Strip Chart (`apps/omniport-desktop/src/components/Telemetry`)
- **`TelemetryRingBuffer` (`telemetryRingBuffer.ts`):**
  - Preallocates contiguous `Float64Array` circular buffers for timestamps, packet rate, throughput, jitter, and CRC error events.
  - Provides zero-allocation `push()` operations during 60 Hz high-speed sampling.
  - Preallocated linearized export buffers (`getLinearData()`) produce contiguous slices formatted for `uPlot` without heap allocations or GC pressure.
- **`TelemetryStripChart.tsx`:**
  - Integrates `uPlot` time-series engine with multiple axes (rate/throughput on primary Y, milliseconds on secondary Y).
  - High-frequency stat bar showing instantaneous packet rate and throughput.
  - `ResizeObserver` dynamic resizing and `requestAnimationFrame` 60 Hz render loop.

### 2.3 Unified Serial Workbench (`SerialWorkbench.tsx`)
- Added a 4-mode navigation tab bar:
  - **Split Workbench:** Top pane runs the 60 FPS Hex Monitor and Logic Strip; bottom pane splits into Visual Topology Canvas and Telemetry Strip Chart.
  - **Hex Monitor:** Dedicated full-viewport Hex/ASCII and modem pin monitor.
  - **Visual Topology:** Dedicated full-screen `@xyflow/react` routing graph.
  - **Telemetry 60Hz:** Dedicated full-screen high-frequency strip chart.

---

## 3. Verification & Benchmarks

### 3.1 Frontend Test Suite (`npm test --prefix apps/omniport-desktop`)
- **19 / 19 tests passed** across 5 test suites:
  - `viewportDecoder.test.js` (3 tests): Binary wire format, bounds validation, and decoder throughput.
  - `hexRenderer.test.js` (5 tests): Viewport calculations, column geometry, and canvas rendering.
  - `logicStripRenderer.test.js` (4 tests): Pin bitmask decomposition, RS-232 signal channels, and 0.605 ms render benchmark.
  - `telemetryRingBuffer.test.js` (4 tests): Sequential ring storage, head wrap-around, and 1,000-cycle export benchmark.
  - `topologyGraph.test.js` (3 tests): Node traits, fault injector parameters, and route edge creation.

### 3.2 Performance Benchmarks (NFR-104)
- **Ring Buffer Throughput:**
  - **Measured Latency:** **6.931 ms** for 1,000 complete ring buffer push and export cycles (~6.9 $\mu\text{s}$ per 60 Hz frame).
  - **Memory Impact:** Zero garbage collection allocations during sliding window updates.
- **Production Bundle (`npm run build`):**
  - TypeScript check (`tsc`) and Vite build (`vite build`) succeeded with 0 errors in **4.10s**.

### 3.3 Workspace Rust Tests & Clippy
- `cargo test --workspace`: **79 / 79 unit/property tests passed**.
- `cargo clippy --workspace -- -D warnings`: **0 warnings**.
- `cargo clippy --manifest-path apps/omniport-desktop/src-tauri/Cargo.toml -- -D warnings`: **0 warnings**.

---

## 4. Architectural Invariants Sign-off
- [x] **FR-202 (Routing Matrix):** Visual interactive topology with endpoints, splitters, mergers, and fault nodes.
- [x] **FR-301 / FR-302 (Fault Injection):** Configurable jitter, delay, and BER on topology nodes and edges.
- [x] **NFR-104 (60 FPS Budget):** Zero-allocation `Float64Array` ring buffer and `uPlot` chart running comfortably within the 16.6 ms budget.
- [x] **Phase 3 Completion:** All Phase 3 steps (3.1, 3.2, 3.3) are 100% complete and verified.
