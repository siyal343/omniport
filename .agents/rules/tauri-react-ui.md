---
trigger: glob
description: "60 FPS rendering and binary IPC rules for the React 19 + Tauri v2 frontend"
globs: "apps/omniport-desktop/src/**/*.ts, apps/omniport-desktop/src/**/*.tsx"
---

# React 19 & Tauri v2 High-Frequency UI Rules
1. **No Packet State in React:** Never store captured serial packets in `useState` or `zustand` arrays. Keep packets in `omniport-capture` mmap files and request only the visible viewport slice (`start_row`, `row_count`) via `invoke<ArrayBuffer>('fetch_viewport_rows')`.
2. **Binary Decoding via DataView:** Parse the binary response from `fetch_viewport_rows` using a browser `DataView` aligned to the ICD wire specification (`docs/ICD.md`).
3. **Canvas/WebGL Over DOM:** Render the Hex/ASCII monitor and Control-Pin (RTS/CTS/DTR/DCD) logic timeline on HTML5 `<canvas>` elements, and use `uPlot` for 60 Hz telemetry strip charts.
