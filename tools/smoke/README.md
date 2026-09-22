# Viewer smoke test and GPU benchmark

Scripts that drive the built app over the Chrome DevTools Protocol, which WebView2 exposes when started with `--remote-debugging-port`. Node 22 or later; no dependencies.

| File | What it is |
| --- | --- |
| `cdp.mjs` | Minimal CDP client (`connect(port)` → `evaluate`, `send`, `on`). Also a CLI: `node tools/smoke/cdp.mjs "<expression>" [port]`. |
| `cdplog.mjs` | Prints the page's console messages and exceptions as they happen. |
| `smoke.mjs` | The CI smoke test (below). |
| `tools.js` | In-page functional checks run by `smoke.mjs`. |
| `bench.js` | In-page GPU benchmark (manual, below). |

## Smoke test (CI, every push)

```bash
pnpm tauri build --no-bundle
cargo build --release -p locus-validate
node tools/smoke/smoke.mjs
```

It generates a two-scan synthetic scene (600,000 points), imports it into a fresh project in a temp folder, and starts `target/release/locus.exe` on that project with software WebGL (SwiftShader), so no GPU is needed. It then runs `tools.js` in the page: rendering, five picks, every measurement kind with its uncertainty, a stale-pick refusal, lasso counts for both depth modes, every cleanup kind followed by undo, every colour mode, clipping, and EDL on and off.

It fails on a failed check, an uncaught exception, or any console error, which includes three.js shader compile and link errors. SwiftShader proves that shaders compile and the pipeline runs. It says nothing about speed.

## GPU benchmark (manual, before each release)

On a machine with a real GPU, open a large project (the 500M-point perf case) with remote debugging on, maximize the window, and run the benchmark:

```bash
LOCUS_OPEN="E:\Claude\scratch\locus\cases\perf-500m.locus" LOCUS_EXAMINER="Benchmark" WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222 --force_high_performance_gpu" target/release/locus.exe
```

```bash
node tools/smoke/cdp.mjs "$(cat tools/smoke/bench.js)"
```

`bench.js` frames an overview and an interior view, lets the adaptive point budget settle, and reports fps, 95th-percentile frame time, the settled budget and node-selection cost for each. Replace `--force_high_performance_gpu` with `--force_low_power_gpu` to measure the integrated GPU on hybrid laptops. Compare against the figures in `docs/phase2-report.md` and record the new ones there or in the release notes. The target is 30 fps or better on both views.
