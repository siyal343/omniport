# OmniPort Release Checklist (Windows installers)

Status: **unsigned installers**. Code signing, notarization and the Tauri updater
are documented as follow-ups below; they are not part of the current CI pipeline.

## Prerequisites

- Node.js 22 and `npm`
- Rust stable toolchain (MSVC target) with `cargo` on `PATH`
- Windows WebView2 Runtime (preinstalled on Windows 10/11; the installer can also
  bootstrap it)
- Internet access on the first bundle run: Tauri downloads WiX (MSI) and NSIS
  into its cache (`%LOCALAPPDATA%\tauri`) automatically. CI runners do this on
  every cold cache.

## Build

```powershell
cd apps/omniport-desktop
npm ci
npx tauri build --bundles msi,nsis
```

`tauri build` runs `npm run build` first (`tsc && vite build`), then compiles the
release binary with the `custom-protocol` feature so `../dist` is embedded, and
finally produces:

| Artifact | Path |
| :--- | :--- |
| Release binary (embedded frontend) | `apps/omniport-desktop/src-tauri/target/release/omniport-desktop.exe` |
| MSI installer | `apps/omniport-desktop/src-tauri/target/release/bundle/msi/OmniPort_0.1.0_x64_en-US.msi` |
| NSIS installer | `apps/omniport-desktop/src-tauri/target/release/bundle/nsis/OmniPort_0.1.0_x64-setup.exe` |

Bundle metadata lives in `apps/omniport-desktop/src-tauri/tauri.conf.json`
(`productName`, `version`, `identifier`, `bundle.icon`). Bump the version there
before a release; the desktop `Cargo.toml` version is only used by the Rust host.

## Verification (required before publishing)

CI job **`Packaged installers + release smoke (windows)`** performs the whole
flow on every push: build both installers, verify the artifacts exist, upload
them as `omniport-windows-installers`, then run the packaged smoke test.

Local equivalent after `npx tauri build`:

```powershell
cd apps/omniport-desktop
node scripts/smoke-packaged.mjs
```

The smoke test launches the **release binary** (the same binary installed by the
MSI/NSIS packages, not the `tauri dev` debug build) with the host forced into its
disconnected state and asserts, over the WebView2 CDP endpoint:

1. the window serves the embedded frontend over the Tauri custom protocol — no
   `http://localhost:5173` Vite dev server;
2. no demo seed (no `DEMO STREAM` badge / `DEMO · VIEWPORT-DERIVED` telemetry);
3. the capture-disconnected banner renders;
4. the Tauri IPC bridge answers and reports the disconnected host;
5. the topology view shows the disconnected-host error instead of fixtures;
6. zero WebView console/page errors.

Screenshots are written to the gitignored `gui-test-screenshots/`
(`packaged_release_disconnected.png`, `packaged_release_topology_error.png`).

Manual spot-check of the *installed* package (recommended for a public release):

1. Run the NSIS installer (unsigned → SmartScreen warning expected).
2. Launch OmniPort from the Start menu; confirm the same five UI states as above.
3. Uninstall via Settings → Apps; confirm `OmniPort` is removed cleanly.

## Signing / notarization follow-ups (not yet implemented)

- **Code signing (Windows):** obtain an EV/OV Authenticode certificate, add it as
  a CI secret, and sign `omniport-desktop.exe`, the NSIS installer and the MSI
  with `signtool`. Without it Windows SmartScreen warns on first run.
- **Tauri updater:** no updater endpoint or signing key is configured; updates
  are manual installer replacement for now. If enabled, use
  `tauri signer generate` and store the private key outside the repo.
- **macOS notarization:** no macOS bundle target exists yet; if added, sign with
  a Developer ID certificate and notarize with `notarytool`.
- **Supply chain:** `cargo audit` / `cargo deny` are planned in Step 7.8;
  installer hashes are not yet published alongside releases.
