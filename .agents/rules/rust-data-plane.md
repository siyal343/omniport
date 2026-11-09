---
trigger: glob
description: "Strict Rust systems-programming invariants for OmniPort crates and Tauri host"
globs: "crates/**/*.rs, apps/omniport-desktop/src-tauri/**/*.rs"
---

# Rust Data-Plane & HAL Rules
1. **No Unwraps in Library Crates:** Use `thiserror::Error` for domain errors across all `crates/omniport-*` libraries. `.unwrap()` and `.expect()` are permitted only in unit tests.
2. **Zero-Cost Mmap Alignment:** Any struct written to `session.idx` (such as `FrameIndexEntry`) MUST be annotated with `#[repr(C)]` and derive `bytemuck::Pod` and `bytemuck::Zeroable`, with a compile-time size assertion (`const _: () = assert!(std::mem::size_of::<FrameIndexEntry>() == 24);`).
3. **Bounded Channels Only:** Never use `tokio::sync::mpsc::unbounded_channel` on serial data paths. Always use bounded channels or ring buffers with explicit backpressure policies.
4. **Script Sandboxing:** All `mlua` instances in `omniport-script` must configure instruction-count hooks so infinite loops terminate within 1 millisecond.
