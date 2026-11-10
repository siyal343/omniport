/**
 * Packaged-release smoke test for OmniPort Windows installers.
 *
 * Launches the `tauri build` release binary (`--no-bundle` output / the binary
 * embedded by the MSI+NSIS installers), which must serve the embedded frontend
 * through Tauri's custom protocol instead of the Vite dev server. Asserts the
 * production-only states: no demo seed, capture-disconnected banner, topology
 * error banner, working IPC bridge and zero console/page errors.
 *
 * Usage: node scripts/smoke-packaged.mjs
 * Env:   OMNIPORT_RELEASE_EXE  override binary path
 *        SMOKE_CDP_PORT        default 9350
 */

import { chromium } from "playwright-core";
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const DESKTOP_DIR = path.resolve(import.meta.dirname, "..");
const REPO_ROOT = path.resolve(DESKTOP_DIR, "..", "..");
const RELEASE_EXE =
  process.env.OMNIPORT_RELEASE_EXE ??
  path.join(DESKTOP_DIR, "src-tauri", "target", "release", "omniport-desktop.exe");
const CDP_PORT = Number(process.env.SMOKE_CDP_PORT ?? 9350);
const SHOT_DIR = path.join(REPO_ROOT, "gui-test-screenshots");
fs.mkdirSync(SHOT_DIR, { recursive: true });

const checks = [];
function record(id, name, pass, notes = "") {
  checks.push({ id, name, pass, notes });
  console.log(`${pass ? "PASS" : "FAIL"} ${id} ${name}${notes ? ` — ${notes}` : ""}`);
}

function killTree(pid) {
  if (!pid) return;
  try {
    execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
  } catch {
    // Already exited.
  }
}

function processSnapshot() {
  try {
    const processes = execFileSync(
      "powershell",
      [
        "-NoProfile",
        "-Command",
        "Get-CimInstance Win32_Process | Where-Object { $_.Name -in @('omniport-desktop.exe','msedgewebview2.exe') } | Select-Object Name,ProcessId,CommandLine | Format-List | Out-String -Width 500",
      ],
      { encoding: "utf8" }
    ).trim();
    let listeners = "netstat unavailable";
    try {
      listeners = execFileSync("cmd.exe", ["/c", `netstat -ano | findstr ${CDP_PORT}`], {
        encoding: "utf8",
      }).trim();
    } catch {
      listeners = `no listener on port ${CDP_PORT}`;
    }
    return `listeners:\n${listeners || "none"}\nprocesses:\n${processes}`;
  } catch (error) {
    return `process snapshot failed: ${error}`;
  }
}

async function waitForHttp(url, timeoutMs = 45000, exited = () => null) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const exit = exited();
    if (exit) {
      throw new Error(`application exited before serving CDP (${JSON.stringify(exit)})`);
    }
    try {
      const response = await fetch(url);
      if (response.ok) return true;
    } catch {
      // Retry until WebView2 exposes the debug endpoint.
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`CDP endpoint ${url} did not respond within ${timeoutMs}ms`);
}

async function connectWebView(exited) {
  await waitForHttp(`http://127.0.0.1:${CDP_PORT}/json/version`, 45000, exited);
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    const pages = browser.contexts().flatMap((context) => context.pages());
    if (pages.length > 0) return { browser, page: pages[0] };
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  throw new Error("No WebView2 page exposed over CDP");
}

async function readDom(page) {
  return page.evaluate(async () => {
    const q = (selector) => document.querySelector(selector);
    const text = (selector) => q(selector)?.textContent?.replace(/\s+/g, " ").trim() ?? null;
    let captureStatus = null;
    if ("__TAURI_INTERNALS__" in window) {
      try {
        captureStatus = await window.__TAURI_INTERNALS__.invoke("capture_status");
      } catch (error) {
        captureStatus = { error: String(error) };
      }
    }
    return {
      href: location.href,
      title: document.title,
      hasTauri: "__TAURI_INTERNALS__" in window,
      demoBadge: text('[data-testid="demo-badge"]'),
      telemetrySource: text('[data-testid="telemetry-source"]'),
      disconnectedBanner: text('[data-testid="capture-disconnected-banner"]'),
      hasHexMonitor: Boolean(q('[data-testid="hex-monitor"]')),
      captureStatus,
    };
  });
}

async function saveShot(page, filename) {
  await page.screenshot({ path: path.join(SHOT_DIR, filename) });
  return `gui-test-screenshots/${filename}`;
}

let appProc = null;
try {
  if (!fs.existsSync(RELEASE_EXE)) {
    throw new Error(
      `release binary not found at ${RELEASE_EXE}; run \`npx tauri build\` first`
    );
  }
  console.log(`--- Launching packaged release binary: ${RELEASE_EXE}`);
  const env = { ...process.env };
  delete env.OMNIPORT_DESKTOP_DEMO;
  delete env.OMNIPORT_DAEMON_ADDR;
  env.OMNIPORT_DESKTOP_DISCONNECT = "1"; // deterministic production disconnected state
  env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = `--remote-debugging-port=${CDP_PORT}`;

  appProc = spawn(RELEASE_EXE, [], { env, stdio: ["ignore", "pipe", "pipe"] });
  let appExit = null;
  let appOutput = "";
  appProc.on("exit", (code, signal) => {
    appExit = { code, signal };
  });
  appProc.stdout?.on("data", (chunk) => {
    appOutput += chunk.toString();
  });
  appProc.stderr?.on("data", (chunk) => {
    appOutput += chunk.toString();
  });

  const { browser, page } = await connectWebView(() => appExit).catch((error) => {
    console.error(`--- smoke launch diagnostics`);
    console.error(`spawned pid: ${appProc.pid}`);
    console.error(`exit: ${JSON.stringify(appExit)}`);
    console.error(`output:\n${appOutput}`);
    console.error(`processes:\n${processSnapshot()}`);
    throw error;
  });
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => consoleErrors.push(error.message));

  await page.waitForSelector('[data-testid="hex-monitor"]', { timeout: 25000 });
  await page.waitForTimeout(1200);
  const dom = await readDom(page);
  const shot = await saveShot(page, "packaged_release_disconnected.png");

  record(
    "P1",
    "embedded frontend served over Tauri custom protocol (no Vite dev server)",
    dom.hasTauri === true &&
      !dom.href.startsWith("http://localhost:5173") &&
      !dom.href.startsWith("https://localhost:5173") &&
      dom.title.includes("OmniPort"),
    `${dom.href} | title=${dom.title}`
  );
  record(
    "P2",
    "release build contains no demo seed",
    dom.demoBadge === null && dom.telemetrySource !== "DEMO · VIEWPORT-DERIVED",
    `demoBadge=${dom.demoBadge} telemetrySource=${dom.telemetrySource}`
  );
  record(
    "P3",
    "capture-disconnected state renders in the packaged app",
    dom.hasHexMonitor === true &&
      dom.disconnectedBanner !== null &&
      dom.disconnectedBanner.includes("CAPTURE DISCONNECTED"),
    `${shot} | banner=${dom.disconnectedBanner}`
  );
  record(
    "P4",
    "Tauri IPC bridge answers from the packaged app (host reports disconnected)",
    dom.captureStatus !== null && typeof dom.captureStatus === "object" && "error" in dom.captureStatus,
    JSON.stringify(dom.captureStatus)
  );

  await page.click('[data-testid="tab-topology"]');
  await page.waitForSelector('[data-testid="topology-error"]', { timeout: 15000 });
  const topologyError = await page
    .locator('[data-testid="topology-error"]')
    .textContent();
  const topoShot = await saveShot(page, "packaged_release_topology_error.png");
  record(
    "P5",
    "topology view surfaces the disconnected host instead of demo fixtures",
    (topologyError ?? "").toLowerCase().includes("disconnect"),
    `${topoShot} | ${(topologyError ?? "").replace(/\s+/g, " ").trim()}`
  );
  record(
    "P6",
    "packaged app produces zero WebView console/page errors",
    consoleErrors.length === 0,
    JSON.stringify(consoleErrors)
  );

  await browser.close();
} catch (error) {
  record("P0", "packaged smoke test executed", false, String(error));
} finally {
  killTree(appProc?.pid);
}

const failed = checks.filter((check) => !check.pass);
console.log("PACKAGED_SMOKE_RESULTS:", JSON.stringify(checks, null, 2));
if (checks.length === 0 || failed.length > 0) {
  console.error("PACKAGED_SMOKE_FAILED");
  process.exit(1);
}
console.log("PACKAGED_SMOKE_OK");
