# ADR-0008: Length-prefixed JSON over TCP control plane (supersedes ADR-0001)

- **Status:** Accepted
- **Date:** 2026-11-10
- **Supersedes:** [ADR-0001](0001-daemon-first-architecture.md)

## Context

[ADR-0001](0001-daemon-first-architecture.md) established daemon-first process
separation (`omniportd` owning sessions, endpoints, and routes independently of
clients) and originally specified gRPC over Unix Domain Sockets or Windows
Named Pipes.

In practice, `omniport-proto` shipped a length-prefixed JSON codec over TCP
(`crates/omniport-proto/src/codec.rs`), while two diverging `.proto` drafts
remained in the repository and `omniport-desktop` initially owned an in-process
capture engine rather than connecting to `omniportd`. During Phase 6 hardening
(Steps 6.4 and 6.5):

- `CaptureStore` (`apps/omniport-desktop/src-tauri/src/capture_store.rs`) placed
  the Tauri host behind a single backing seam with `DaemonCaptureStore`
  connecting to `omniportd` over `omniport_proto::RpcClient` as the default in
  shipping release builds (`FixtureCaptureStore` is isolated to debug/demo use
  via `OMNIPORT_DESKTOP_DEMO=1`).
- Both legacy `.proto` files (`proto/omniport.proto` and
  `crates/omniport-proto/proto/omniport.proto`) were deleted so
  `crates/omniport-proto/src/{types.rs,codec.rs}` and `docs/ICD.md` §3 form the
  single schema and wire-format owner.

## Decision

1. **Daemon-first separation stands.** `omniportd` (`DaemonEngine`) owns active
   virtual serial endpoints, `Topology` / `RoutingGraph`, and the `Session`
   dual-mmap capture store (`MmapCaptureEngine`) independently of any client
   process, so closing `omniport-cli` or `omniport-desktop` never drops active
   ports or routes.
2. **Shipped control-plane transport is length-prefixed JSON over TCP.**
   `omniportd`, `omniport-cli`, and `omniport-desktop` (`DaemonCaptureStore`)
   communicate via `omniport_proto::RpcClient` using `[len: u32 big-endian][payload: UTF-8 JSON]`
   frames over TCP (`127.0.0.1:50051` by default, configurable via
   `omniportd --bind`, `omniport --server`, and `OMNIPORT_DAEMON_ADDR`), capped at
   `MAX_FRAME_SIZE = 16 MiB`, as specified normatively in `docs/ICD.md` §3.
3. **Two-hop viewport delivery for the desktop UI.** Over the TCP control plane,
   `Request::ReadViewport` returns `Response::ViewportBatch { bytes }` with the
   ICD §2 batch base64-encoded inside JSON and capped at
   `MAX_VIEWPORT_BATCH_BYTES = 8 MiB` raw (`Error { code: 413, ... }` above the
   cap). Between the Tauri host (`DaemonCaptureStore`) and the WebView,
   `fetch_viewport_rows` transfers the validated ICD §2 byte buffer as a raw
   binary `ArrayBuffer` via `tauri::ipc::Response` (ADR-0004, ADR-0007).
4. **UDS / Named Pipe remains a future local-IPC option.** Because
   `omniport_proto::codec::{read_message, write_message}` is generic over
   `AsyncReadExt` / `AsyncWriteExt`, migrating local transport from loopback TCP
   to Unix Domain Sockets (POSIX) or Named Pipes (Windows) remains an additive
   transport swap without changing `Request` / `Response` framing or semantics.

## Consequences

- **No `.proto` / `protoc` / `tonic` toolchain dependency** and no second schema
  copy that can drift from `crates/omniport-proto/src/types.rs`.
- **Cross-platform parity and remote operation:** TCP framing works identically
  across Windows, Linux, and macOS and supports remote headless control without
  additional tunneling.
- **Wire overhead and connection hardening:** base64 expands `ViewportBatch`
  payloads by ~4/3 on the daemon control plane (bounded by
  `MAX_VIEWPORT_BATCH_BYTES`), and TCP loopback provides neither OS
  peer-credential checks nor per-connection read timeouts / connection caps yet
  (tracked as a remaining hardening item in `docs/AUDIT_2026-11-10.md` §8).
- **Partial GUI control-plane coverage today:** `apps/omniport-desktop/src-tauri`
  currently registers `fetch_viewport_rows` and `capture_status`; interactive
  topology mutations from the desktop UI to `omniportd` (`GetTopology`,
  `CreateVirtualPair`, `AddRoute`, `Remove*`) are not yet wired to Tauri
  commands (`omniport-cli` exercises the full control-plane surface).
