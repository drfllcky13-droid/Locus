# Phase 2 report: point-cloud engine

Date: 2026-09-22. Hardware: RTX 3070 Ti and Intel UHD 770 (i7-12700K, 64 GB), WebView2 153. Canvas 1900×1341 px (the window maximized, less the side panels).

## Acceptance criteria

| Criterion | Result |
|---|---|
| 500M-point scene stays above 30 fps on a mid-range GPU | **RTX 3070 Ti: 47 fps overview, 65 fps interior. UHD 770: 51 / 40 fps**, with the adaptive budget settling at 2.5–3.2M points. A real mid-range card has **not** been measured, so this stays under Blocked (as agreed). |
| Picked-point coordinates match the source file within 1 mm | **Pass.** `crates/locus-octree/tests/scene.rs::picks_far_from_the_origin_resolve_to_the_source_within_a_millimetre` imports a georeferenced LAS about 500 km / 4,400 km from the origin and resolves served points back to the source to better than 1 µm. `app/src/viewer3d/measureFormat.test.ts` checks the UI shows those coordinates within 0.5 mm. |
| Every cleanup is undoable and audit-logged | **Pass.** `scene.rs::box_delete_is_undoable_and_never_touches_the_source`, `stale_picks_and_altered_cleanup_files_are_refused`, and `locus-core/tests/state.rs`. Also checked in the running app: lasso delete with Ctrl+Z / Ctrl+Y, and box, outlier and voxel cleanup, each undone. |

## The 500M-point scene

Generated with `locus-validate gen-scene --scans 20 --points-per-scan 25000000 --seed 7`: a 40 m × 30 m × 5 m room with pillars and crates, scanned from 20 posed stations with 1 mm range noise and 0.3 % no-return beams. That gives 500,000,000 records and 498,500,832 valid points in a 6.28 GB E57.

Imported with `locus-validate import`, which runs the app's own preview, commit and octree path:

| Step | Time |
|---|---|
| Preview (read every point, SHA-256) | 67.8 s |
| Copy into the project (hashed, verified) | 23.7 s |
| Octrees, 20 scans (28,243 nodes, 16 GB) | 266 s (about 13 s per 25M-point scan) |
| **Peak memory** for the whole import | **550 MB working set, 715 MB committed** |

## Frame rate and node selection (full 500M-point scene)

Measured with `__locus.benchmark(20)` after a 10 s warm-up, orbiting the camera continuously. The script is `E:\Claude\scratch\locus\bench.js`, run over WebView2 DevTools.

| | RTX 3070 Ti overview | RTX 3070 Ti interior | UHD 770 overview | UHD 770 interior |
|---|---|---|---|---|
| Frame rate | 47.5 fps | 65.0 fps | 51.0 fps | 40.4 fps |
| p95 / max frame | 22.4 / 26.5 ms | 16.7 / 23.5 ms | 20.8 / 21.6 ms | 26.1 / 27.6 ms |
| Point budget (adaptive) | 29.0M | 30.0M (cap) | 2.54M | 3.16M |
| Points drawn | 29.0M | 25.2M | 2.54M | 3.16M |
| **Node selection, CPU, avg / max** | **0.97 / 2.3 ms** | **0.88 / 1.6 ms** | 0.17 / 0.8 ms | 0.17 / 0.6 ms |
| Nodes visited / selected | 4,999 / 1,186 | 3,506 / 1,433 | 483 / 138 | 387 / 156 |

Node selection is a priority-queue traversal from each scan's root: it costs under 2.5 ms per frame over 28,243 nodes and 20 scans even at a 30M-point budget. It doesn't need to move off the main thread or into Rust.

## Spike items and review items

1. **Adaptive budget**: `app/src/viewer3d/budget.ts`. It starts at 10M, and the table above shows it settling at about 29M on the 3070 Ti and about 3M on the UHD 770.
2. **Budgeted GPU uploads**: at most 4 MB of new nodes per frame (`pointcloud.ts`). Nodes average about 0.6 MB.
3. **Four node requests in flight**: `pointcloud.ts`, over the `locus://` protocol.
4. **EDL with correct colour**: linear pipeline, half-float target, encoded once. Checked by reading pixels back: the background reads (30,31,34), exactly `#1e1f22`.
5. **Redraw on demand, including node arrival**: node loads call `requestRender`, and the loop idles otherwise.
6. **Picking resolves in Rust from f64 data**: see the acceptance row above. Measurement commands re-resolve every pick on the Rust side, and picks from a stale view are refused.
7. **GPU exports**: `NvOptimusEnablement` and `AmdPowerXpressRequestHighPerformance` are exported (checked with `dumpbin /exports`), WebView2 gets `--force_high_performance_gpu`, and Help → About shows the GPU WebGL is actually using. Caveat: the exports don't reach WebView2's GPU process (see DECISIONS), and this is untested on a hybrid-graphics laptop.
8. **Full resolution around the cursor for measurement**: `lod.ts` gives nodes on the cursor ray top priority down to the leaves (`viewer.test.ts`).

## Bugs found only by running the app

Unit tests passed throughout. These were found by driving the built app and reading WebView2's console over DevTools:

- The GLSL 3 point shaders didn't compile: three.js r186 has no `gl_FragColor` alias for them. Nothing was drawn.
- Colours were converted twice (sRGB render target), which washed them out. This was the spike's problem too.
- Nodes waiting in the upload queue were requested again. Orphaned copies stayed in the scene, which leaked GPU memory and corrupted picks (a pick could hit an orphan with the wrong material).

These are the kind of failures a WebGL smoke test would catch. The DevTools scripts (`cdp.mjs`, `cdplog.mjs`, `bench.js`, `tools.js` in `E:\Claude\scratch\locus\`) could become a scripted smoke test later. CI has no GPU, so for now it would be a manual pre-release step.

## Not done or limited

- Mid-range GPU measurement (Blocked).
- Picking needs the node's point index below 2^20 and at most 4,095 nodes loaded. Nodes hold up to 50k points and eviction keeps the loaded count well under that limit, so neither is reached in practice.
- Lasso delete removes points at every depth by design (docs/methods/cleanup.md).
- Outlier removal on large regions is CPU-bound (single-threaded k-d tree per tile). It took seconds on 8M points and would take minutes on 500M. Parallelising it with rayon is the obvious upgrade.
