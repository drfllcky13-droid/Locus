# Viewer smoke tests and GPU benchmark

| File | What it is |
| --- | --- |
| `viewer-smoke.mjs` | CI smoke test: the built viewer in headless Chromium (below). |
| `fixture/` | Recorded backend answers and octree nodes the CI test serves. |
| `capture.mjs` | Re-records `fixture/` from the real app. |
| `app-smoke.mjs` | Full-app smoke test on WebView2 (manual). |
| `app.mjs` | Launches the app on a fresh synthetic project; used by the two above. |
| `tools.js` | In-page functional checks run by `app-smoke.mjs`. |
| `bench.js` | In-page GPU benchmark (manual). |
| `cdp.mjs` | Minimal DevTools client (`connect(port)` → `evaluate`, `send`, `on`); also a CLI: `node tools/smoke/cdp.mjs "<expression>" [port]`. |
| `cdplog.mjs` | Prints the app's console messages and exceptions as they happen. |

## Viewer smoke test (CI, every push)

```bash
pnpm -C app build
pnpm -C app exec playwright install chromium
node tools/smoke/viewer-smoke.mjs
```

Loads `app/dist` in headless Chromium with software WebGL (SwiftShader), so no GPU or desktop session is needed. The Tauri IPC and the `locus://` node protocol are mocked: commands are answered from `fixture/ipc.json` and nodes from `fixture/nodes/`, recorded from the real app on a 2 × 60,000-point synthetic scene. A command the fixture doesn't cover fails the test.

It renders the scene, reads the pixels back after each step (RGB, intensity and elevation colouring, EDL off, clip box and plane), and picks a point. It fails on any shader compile or link error, console error or uncaught exception, if nothing is drawn, or if the canvas is blank (under 1 % of pixels differ from the background, with the grid hidden).

It runs on the Linux CI job. The real app can't be driven there (Tauri uses WebKitGTK on Linux, which has no DevTools port). On the Windows runner, WebView2 ignored the debugging arguments, and there is no interactive desktop.

**When a command's response or the node format changes**, re-record the fixture on a Windows machine with a built app, then commit it:

```bash
pnpm tauri build --no-bundle
cargo build --release -p locus-validate
node tools/smoke/capture.mjs
```

## Before each release (manual, real GPU)

### Full-app smoke test

```bash
node tools/smoke/app-smoke.mjs
```

Runs the real app (WebView2 and the Rust backend) on a fresh synthetic project and runs `tools.js` in it: rendering, picks resolved in Rust, every measurement kind with its uncertainty, a stale-pick refusal, lasso counts in both depth modes, every cleanup kind followed by undo, colour modes, clipping and EDL. It fails on a failed check, an exception or any console error. Pass `--swiftshader` to render on software WebGL instead of the GPU.

### GPU benchmark

Open a large project (the 500M-point perf case) with remote debugging on, maximize the window, and run the benchmark:

```bash
LOCUS_OPEN="E:\Claude\scratch\locus\cases\perf-500m.locus" LOCUS_EXAMINER="Benchmark" WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222 --force_high_performance_gpu" target/release/locus.exe
```

```bash
node tools/smoke/cdp.mjs "$(cat tools/smoke/bench.js)"
```

`bench.js` frames an overview and an interior view, lets the adaptive point budget settle, and reports fps, 95th-percentile frame time, the settled budget and node-selection cost for each. To measure the integrated GPU on a hybrid laptop, replace `--force_high_performance_gpu` with `--force_low_power_gpu`. Compare against `docs/phase2-report.md`, and record the new figures there or in the release notes. The target is 30 fps or better on both views.
