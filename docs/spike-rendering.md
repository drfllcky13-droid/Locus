# Rendering spike: can the Tauri webview render large point clouds? (Phase 2, SPEC section 5)

Date: 2026-09-22. Status: **recommend staying in the webview (WebGL2)**; awaiting Addison's decision.

## Question

Can the webview both stream point data from Rust and draw enough points per frame, with eye-dome lighting (EDL), to meet "500M-point scene above 30 fps on a mid-range GPU"? With level of detail, only a budget of points is drawn each frame, so the real question is the budget the webview can sustain, and whether streaming chunks in causes hitches.

## Setup

- Release build of the app with the `spike` feature (`src-tauri/src/spike.rs`, `app/spike/main.ts`), using the production asset protocol and CSP. The window was maximized at **2560×1341 px** (DPR 1).
- Synthetic scene: 50 tiles of 1M points each (terrain plus walls), float32 xyz relative to a local origin plus RGB8, 15 bytes per point, 750 MB in total. It is generated on first run into `target/fixtures/` and never committed.
- Three.js `Points` (WebGL2 via ANGLE on D3D11), 2 px points, and a full-screen EDL pass (8-neighbour log-depth obscurance). The camera orbits continuously, and frame intervals come from `requestAnimationFrame`, so rates are capped by the 240 Hz display.
- WebView2 153 (Chromium 153). WebGPU is **available** in this webview.
- GPUs: **RTX 3070 Ti** (upper mid-range) and **Intel UHD 770** integrated graphics (well below mid-range), forced with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--force_low_power_gpu` for that run only. WebGL's `powerPreference: "low-power"` alone was ignored.

## Results

### Frame rate by point budget (EDL on; EDL off is within noise of these)

| Points drawn per frame | RTX 3070 Ti: fps | RTX 3070 Ti: p95 frame | UHD 770: fps | UHD 770: p95 frame |
|---|---|---|---|---|
| 1M | 240 (display cap) | 4.3 ms | 91 | 12.8 ms |
| 5M | 240 (display cap) | 4.3 ms | 31 | 62.6 ms |
| 10M | 132 | 8.5 ms | 17 | 117 ms |
| 20M | 103 | 12.7–16.7 ms | 9 | 225 ms |
| 30M | 71 | 17–25 ms | 6 | 168–329 ms |
| 50M | 43 | 34–46 ms | 3.7 | 550 ms |

Throughput is about 1.3 billion points per second on the 3070 Ti and about 0.17 billion on the UHD 770. The EDL pass costs under 0.5 ms at this resolution.

### Draw-call overhead (the octree case)

The same points were split into 20k-point pieces sharing the chunks' GPU buffers, as an octree's nodes would be:

| | RTX 3070 Ti, EDL | UHD 770, EDL |
|---|---|---|
| 10M points in 500 draws | 209 fps, p95 8.4 ms | 17 fps |
| 20M points in 1,000 draws | 107 fps, p95 12.6 ms | 9 fps |

Draw count makes no measurable difference, so per-draw overhead through ANGLE is not a constraint at octree scale.

### Streaming (Rust → webview → GPU)

| | Custom-protocol `fetch` | Binary `invoke` |
|---|---|---|
| One 15 MB chunk at a time | 144 MB/s (104 ms per chunk) | 146 MB/s (102 ms per chunk) |
| Four in flight while rendering (fetch) | **235 MB/s** effective, about 15.7M points/s | — |

With all 50M points streamed in while rendering with EDL on the 3070 Ti, the first chunk was visible after 133 ms and everything had loaded after 3.2 s. The worst frame was 46.7 ms, with none over 50 ms and a p95 of 21 ms. Those spikes come from uploading a whole 15 MB chunk to the GPU in one frame. Octree nodes are about 1 MB and uploads can be spread across frames, so real hitches will be smaller.

The two transport methods perform the same. About 145 MB/s per request is WebView2's copy overhead, not disk speed.

## Against the decision rule (PROGRESS.md, Phase 2 step 1)

| Criterion | Result | Pass |
|---|---|---|
| At least 10M points with EDL at 30 fps or more on the 3070 Ti, p95 under 16 ms | 132 fps, p95 8.5 ms | ✅ |
| Scales to at least 5M points on a mid-range card | 1.3 billion points/s measured. A mid-range card at 0.45–0.7× the 3070 Ti (GTX 1660 Super to RTX 3060) would draw 10M points at an estimated 60–90 fps. | ✅ (estimate) |
| Streaming above 100 MB/s | 145 MB/s single, 235 MB/s parallel | ✅ |
| No hitches over 50 ms while streaming | 0 frames over 50 ms (worst 46.7 ms) | ✅ (just) |

## Recommendation

**Stay in the webview with WebGL2. Do not switch to native wgpu now.**

- The GPU's vertex throughput is the limit, not the webview: draw calls and the EDL pass are nearly free.
- WebGPU is already available in WebView2, so compute-shader point rendering (the fastest known approach) remains possible later without leaving the webview.
- Native wgpu would add a second rendering stack alongside Three.js (diagrams, models, gizmos, measurement) and complicate compositing, for gains we don't need at this target.

## What Phase 2 must build in, because of these numbers

1. **An adaptive point budget.** Start at about 10M points and adjust from measured frame time. Integrated graphics, common on field laptops, only hold 30 fps at about 5M points and need a lower default.
2. **Budgeted GPU uploads.** Cap the bytes uploaded per frame and use about 1 MB octree nodes, so streaming stays well under the 50 ms hitch limit.
3. **Four or more requests in flight** for node loading. A single request tops out at about 145 MB/s.
4. **Correct color-space handling in the EDL composite.** The spike's pass double-encodes sRGB, which washes colors out; that's cosmetic and doesn't affect timing, but it must be fixed.
5. **The redraw-on-demand viewport** (DECISIONS 2026-09-22): a render is requested whenever a node finishes uploading.

## Limits of this spike

- It measured one upper-mid-range and one integrated GPU. The mid-range figure is an estimate from measured throughput, not a measurement on a mid-range card. **Acceptance testing needs a real mid-range card** (for example an RTX 3060 or GTX 1660).
- The data is synthetic, and the full 500M-point scene with LOD selection is not tested yet; CPU cost of node selection will be measured when the octree exists.
- rAF timing is capped at 240 Hz, so results at the cap only show "at least".
- The low-power flag and WebView2's GPU choice may behave differently on laptops with switchable graphics.

## Reproducing

```
LOCUS_SPIKE=1 pnpm tauri build --no-bundle --features spike --config src-tauri/tauri.spike.conf.json
LOCUS_SPIKE_QUERY="label=high-performance" target/release/locus.exe
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--force_low_power_gpu LOCUS_SPIKE_QUERY="label=igpu" target/release/locus.exe
```

Results are written to `target/fixtures/spike-results-<label>.json`.
