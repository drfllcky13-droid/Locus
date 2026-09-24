# Progress

Current phase: **Phase 2 complete** (pending the latest CI run). The mid-range GPU measurement is still open under Blocked, as agreed. Phases 0 and 1 are complete.

## Phase log

### Phase 0: Scaffold and CI

Plan:
1. Root Cargo workspace (`resolver = "2"`) with stub crates: locus-core, locus-io, locus-octree, locus-register, locus-analysis, locus-photo, locus-report, locus-validate (bin).
2. `src-tauri/` Tauri 2 shell as a workspace member; depends on locus-core only for now. Native menu bar (File / Edit / View / Help) built with the Tauri menu API.
3. `app/` React + TypeScript + Vite frontend. One component mounts a Three.js renderer: empty scene, Z-up camera, grid + axes, orbit controls, resize handling. Vitest for `pnpm test`, `tsc --noEmit` for `pnpm typecheck`.
4. Root pnpm workspace (`app` is the member) so root scripts `pnpm tauri dev`, `pnpm test`, `pnpm typecheck`, `pnpm lint` all work from the repo root.
5. Lint/format: `rustfmt.toml`, workspace clippy lints, ESLint (flat config) + Prettier for `app/`, `.editorconfig`.
6. `.github/workflows/ci.yml`: matrix windows-latest + ubuntu-latest; rustfmt check, clippy `-D warnings`, cargo test, pnpm typecheck/lint/test, and a `tauri build --no-bundle` compile check.
7. Keep empty dirs from the spec layout (`viewer/`, `assets/`) out until a phase needs them.

Acceptance criteria:
- [x] `pnpm tauri dev` opens a window with an empty 3D viewport and a menu bar (verified 2026-09-22 by window capture: File/Edit/Help menu, Z-up grid and axes)
- [x] CI is green (Windows + Linux): run 35776556905 on d109b02, all jobs passed (2026-09-22)
- [x] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck`, `pnpm lint`, `pnpm format:check`, `pnpm test` pass locally on Windows

Local setup done: rustup stable (1.98.1, MSVC) and VS 2022 Build Tools (C++ workload) installed via winget. Rust lives in E:\Rust.

Follow-ups after review (2026-09-22): all crates confirmed `publish = false` (inherited from the workspace); cargo-deny license check added to CI to enforce CLAUDE.md rule 7.

### Phase 1: Project model, evidence integrity, import (done 2026-09-22; CI green on a4577a7)

Plan:
1. **locus-core: project bundle.** `Project::create(dir, name, examiner)` / `Project::open(dir)` on a `.locus` folder with `project.sqlite`, `evidence/`, `derived/`, `assets/`. rusqlite with bundled SQLite. Every mutation is a transaction, so there is no separate "save" step (see DECISIONS). Schema version stored in a `meta` table.
2. **locus-core: audit log.** Append-only `audit_log` table; each row stores `prev_hash` and `hash = SHA-256(prev_hash ‖ seq ‖ timestamp ‖ actor ‖ action ‖ details_json)`. The first row chains from a fixed genesis value. `open()` re-walks the chain and refuses to open (typed `Tampered { seq }` error) on any edit, deletion, reorder or insertion. Tail truncation: the chain head (seq + hash) is also mirrored into `meta` on every append and checked on open; that catches truncation done with SQL, while the limits of a self-contained log are written down in `docs/methods/audit-log.md`.
3. **locus-core: evidence.** `import_evidence(src)` opens the source read-only, streams it through SHA-256 while copying into `evidence/<sha256-prefix>_<original name>`, re-hashes the copy, marks the copy read-only, records the original path, size, source mtime and hash, and appends an audit entry. Importing the same bytes twice is refused, naming the existing record. `verify_evidence()` re-hashes every stored file.
4. **locus-io: format readers.** One module per format, each exposing a streaming `inspect(path, progress) -> ImportSummary` (format, true point/vertex count from reading every record, bounds, per-scan name, pose as a 4x4 and point count, attributes present, declared unit if any, images/panoramas), plus a streaming point iterator for Phase 2 to reuse. Crates: `e57` (E57, multi-scan, poses, embedded images), `las` with `laz` (LAS/LAZ), `tobj` (OBJ), `gltf` (glTF/GLB), `kamadak-exif` + `imagesize` (JPEG/PNG EXIF and dimensions). PLY, PTS and XYZ get small hand-written streaming readers (ASCII and binary little/big endian for PLY). Embedded E57 panoramas are written to `derived/panoramas/`.
5. **Units.** E57 is meters by definition. LAS/LAZ unit comes from the GeoTIFF/WKT CRS when present. PLY/PTS/XYZ/OBJ/glTF declare nothing (glTF is meters by spec). When a format declares no unit, the import dialog requires the examiner to pick one (no silent default); the choice is stored with the scan and audit-logged. Source coordinates are never rescaled on disk.
6. **Import orchestration (locus-io).** Two steps: *preview* reads the source read-only and returns hash plus summary; *commit* copies it into the project, checks the copy's hash equals the preview hash, and writes scans/images rows plus one audit entry. A file whose format can't be parsed is refused before copying.
7. **Tauri + UI.** Commands: `project_create`, `project_open`, `import_preview` (streams progress over a channel), `import_commit`, `evidence_list`. File menu gets New Project…, Open Project…, Import… (tauri-plugin-dialog for pickers). Import dialog shows file name, size, SHA-256, format, point count, per-scan table, images, a unit picker when required, and help text explaining that native scanner formats need vendor software: export E57 from the scanner's own software. Side panel lists evidence with hashes.
8. **Tests.** core: create/open round trip; tamper cases (edit payload, delete a middle row, reorder, truncate tail) are each detected on open; source hash and mtime unchanged after import; stored copy is byte-identical and read-only; duplicate refused. io: each reader against small fixtures generated inside the test (E57 with two posed scans and an embedded panorama written with the `e57` writer; LAS and LAZ via `las`; PLY ASCII/binary, PTS, XYZ, OBJ, glTF and EXIF images built in code). A proptest round-trip for the text readers. Memory: `#[ignore]` test that generates a 100M-point E57 into `target/fixtures/` (never committed; see DECISIONS) and checks peak process memory stays under 4 GB while importing.

Acceptance criteria:
- [x] Importing a file never modifies it: `locus-core/tests/integrity.rs::import_never_modifies_source` and `locus-io/tests/readers.rs::preview_then_commit_leaves_source_untouched` compare SHA-256 and mtime before and after import. Also checked by hand in the running app (sample XYZ: source and stored copy both hash to `0bdc7212…90d0e0`; the stored copy is read-only).
- [x] Tampering with the audit log is detected on open: 8 tests in `integrity.rs` cover an edited entry, edited actor/timestamp, a deleted middle entry, reordered entries, a truncated tail, a forged head, and an appended forged entry. There is also a proptest that edits any single field of any entry.
- [x] A 100M-point E57 imports without exceeding 4 GB RAM: `locus-io/tests/e57_memory.rs` (release build) generates 4 × 25M-point scans (0.89 GB), then previews and imports them in 13 s. Peak working set was 72 MB and peak committed 68 MB, of which 64 MB is a deliberate probe that proves the meter works. It runs in CI on Linux.

What was built:
- `locus-core`: `.locus` project bundle (create/open), hash-chained audit log with append-only triggers and a mirrored head, evidence import (read-only source, hashed copy, re-hash, read-only stored copy, duplicate refusal, unit rules), `verify_evidence`, and units with exact factors. Method note: `docs/methods/audit-log.md`.
- `locus-io`: streaming readers for E57 (multi-scan, poses, invalid points, spherical, embedded images and their extraction), LAS/LAZ (CRS unit detection), PLY (ASCII, binary LE/BE, mesh vs cloud), PTS (multi-block), XYZ, OBJ, glTF/GLB, JPEG/PNG with every EXIF field, and file-type checks by leading bytes. Two-step `preview` then `commit`.
- Tauri: `project_create`, `project_open`, `import_preview`, `import_commit` and `evidence_verify` commands, with progress over channels. The File menu has New / Open / Import Evidence / Verify Evidence. E57 images are extracted to `derived/e57-images/<id>/` after commit.
- UI: welcome screen, new/open project dialogs (the examiner name is required), import dialog (SHA-256, size, format, point count, per-scan/mesh/image tables with extents in the source unit, EXIF, warnings, required unit picker, native-format help text), and an evidence panel with hashes and the audit head.
- Tests: 17 core, 21 io (plus 1 ignored memory test), 9 frontend (including a mocked-IPC render test of the import dialog).

Differences from the plan:
- The streaming point iterator for Phase 2 was not built. Readers compute counts and bounds in one pass, and Phase 2 will extract the iterator from them when the octree builder needs it.
- There is no `evidence_list` command; every command returns the refreshed project info instead.
- E57 records are decoded from the raw reader because of an upstream bug in the `e57` crate's simple reader (see DECISIONS).

Follow-up after review (2026-09-22): opening a project now re-hashes every evidence file and reports changed, missing and unrecorded files. The result is logged in `project.opened` and shown as a warning in the evidence panel. Tests: `opening_rehashes_evidence_and_reports_changes` and `opening_reports_missing_and_unrecorded_evidence_files`.

Open issues:
- Opening a project re-hashes all evidence, which takes about 1 s per GB. That's acceptable, and progress is shown, but very large cases will open slowly.
- OBJ files are read fully into memory (`tobj`); this is acceptable for meshes and is marked with a `ponytail:` comment.
- LAS files that name only an EPSG code make the examiner choose the unit; resolving it needs an EPSG table.
- The `e57` simple-reader bug should be reported upstream.

### Phase 2: Point cloud engine (approved 2026-09-22; CI green on c85b65d, with the viewer smoke test)

Plan (step 1 gates the rest; results go to Addison before any feature work):

1. **Rendering spike (SPEC section 5).** Prove, or disprove, that the Tauri webview can stream and render a large point cloud at the target frame rate.
   - *Target:* 500M-point scene above 30 fps on a mid-range GPU. With LOD only a budget of points is drawn per frame, so the question is whether the webview can draw a realistic budget (roughly 5-20M points) with eye-dome lighting at display resolution above 30 fps, and keep receiving chunks fast enough without frame hitches.
   - *Rust side:* synthetic chunk file (1M points per chunk, float32 xyz relative to a local origin plus RGB8, 15 bytes per point) generated into `target/fixtures/`. It is served to the webview two ways so they can be compared: a custom URI scheme read with `fetch()` into an ArrayBuffer, and binary `invoke` responses.
   - *Webview side:* a separate `spike.html` entry using Three.js `Points` (WebGL2) with an eye-dome-lighting pass. It measures chunk transfer throughput (MB/s, per-chunk latency), time to first frame, steady-state fps and p95/max frame time at 1, 5, 10, 20 and 30M visible points with EDL off and on, and the worst frame time while chunks stream in. It also records renderer, GPU, resolution and WebGPU availability.
   - *Hardware:* RTX 3070 Ti (upper mid-range) at 2560×1440, plus a low-power request that may land on the Intel UHD 770 as a below-mid-range reference.
   - *Decision rule:* stay in the webview if at least 10M points with EDL hold 30 fps on the 3070 Ti with headroom (below 16 ms p95), or if the numbers scale to at least 5M on a mid-range card, and if streaming sustains more than 100 MB/s without hitches over 50 ms. Otherwise switch the point renderer to native wgpu now.
   - Spike code sits behind a `spike` cargo feature and a separate HTML entry so it never ships.
   - **Result (2026-09-22):** every decision-rule criterion passes. RTX 3070 Ti: 10M points with EDL at 132 fps (p95 8.5 ms). 500 or 1,000 draws cost the same as 10. Streaming runs at 145 MB/s per request and 235 MB/s with 4 in flight, with no frame over 50 ms. Intel UHD 770 holds 30 fps at 5M. **Recommendation: stay in the webview (WebGL2); WebGPU is available later.** Full report: `docs/spike-rendering.md`.
2. **Point streaming in locus-io.** `for_each_point(path, scan)` for every scan format: scan-local position (source units), RGB and intensity if present, and the record's index in the source file. The octree builder and every cleanup read through this.
3. **locus-octree: out-of-core builder, own format.** One octree per scan, in scan-local coordinates converted to meters. The pose is kept separate, so registration (Phase 3) only changes a matrix and never rebuilds. Stored in `derived/octree/<evidence>-<scan>/`.
   - A counting pass fills a 128³ grid, and chunks are chosen as the largest octree nodes holding at most 5M points.
   - A distribution pass walks each point down the upper levels, where it is kept if its grid cell is empty (128³ grid sampling per node, Potree 1 style). Otherwise it is appended to its chunk's temp file.
   - Each chunk is then indexed in memory the same way, top-down.
   - Every point lives in exactly one node, so the union of nodes on a path down to a leaf is full resolution.
   - A node stores f64 xyz, a u32 source index, RGB8 and u16 intensity. The hierarchy (bounds, counts, spacing, byte ranges) is JSON. Memory is bounded by the chunk size, not the scan size.
   - Runs as a background task with progress after import (or on open if missing), recorded in an `octrees` table and audit-logged.
4. **Serving nodes.** A `locus://` async protocol returns a node as f32 xyz relative to the node's minimum corner, plus intensity and RGB, with removed points filtered out. Georeferenced coordinates never reach the GPU. The render origin is the centre of the project's data, and object matrices are computed in f64 in JS.
5. **Renderer (app/src/viewer3d).**
   - LOD selection by projected node size under a point budget. It is a pure function, tested, and timed on the 500M-point scene.
   - **Adaptive budget** (spike item 1): starts at 10M points and follows measured frame time.
   - **Budgeted GPU uploads**, at most about 4 MB per frame (item 2).
   - **Four node requests in flight** (item 3).
   - **EDL with correct color space** (item 4).
   - **Redraws only when something changes**: camera, node arrival, tool state (item 5).
   - Its own point shader: RGB, intensity or elevation coloring, and point size from node spacing (size by LOD).
   - Clip box (moved and scaled with gizmos) and clip plane (axis and offset).
6. **Picking and measurement (item 6).** A GPU pick pass renders a small window around the cursor into an RGBA8 target that encodes node slot and point index, and nothing else. Rust resolves the id to the stored f64 point and applies unit and pose in f64.
   - Measurements are distance, angle, polygon area (on a least-squares plane, with residual) and height above a fitted plane.
   - The math lives in `locus-analysis` as pure functions with hand-checked values, property tests and propagated uncertainty. The per-point σ is a project setting, default 2 mm, and is stated with every result.
   - Measurements are stored in SQLite and audit-logged. A method note goes in `docs/methods/measurement.md`.
7. **Cursor-focused refinement (item 8).** While a measurement tool is active, nodes intersecting the cursor ray are loaded down to the leaves first, whatever their screen size, so snapping uses full-resolution points.
8. **Cleanup: box delete, lasso delete, statistical outlier removal, voxel downsample.**
   - Each operation stores its full parameters in SQLite (the lasso stores its polygon and f64 view-projection matrix) and writes a roaring bitmap of removed source indices, with its SHA-256, to `derived/cleanup/`. Source data is never touched.
   - The removed set is the union of active operations. Undo and redo toggle an operation, and each change is audit-logged. Ctrl+Z / Ctrl+Y and a list with checkboxes.
   - Outlier removal and voxel downsampling run tile by tile over the octree with a margin, so memory stays bounded.
9. **GPU choice (item 7).** Export `NvOptimusEnablement` and `AmdPowerXpressRequestHighPerformance` from locus.exe. WebGL renders in WebView2's own GPU process, which those exports don't reach, so also pass `--force_high_performance_gpu` to WebView2. Help → About shows the GPU WebGL is actually using.
10. **500M-point scene and report.** `locus-validate gen-scene` writes synthetic multi-scan E57 scenes (rooms: floor, walls, objects) at scanner-like density. Import 500M points (20 scans × 25M), build the octrees, and measure:
    - frame rate and budget on the RTX 3070 Ti and UHD 770;
    - the CPU cost of node selection;
    - build time and peak memory.

    Results go in `docs/phase2-report.md`. "Measured on a real mid-range card" stays under Blocked.

Tests:
- Octree: every point exactly once, node limits, determinism, bounded memory.
- Precision (item 6): a georeferenced LAS far from the origin (e.g. 500 km, 4,400 km). The coordinate resolved for a pick equals the source within 1 mm, and the UI's formatted value keeps the millimetres.
- Measurement math: hand-checked values, property tests, uncertainty propagation.
- Cleanup: each operation removes the right points, source hash unchanged, undo/redo restores and is logged, bitmap tamper detected.
- Frontend: LOD selection, budget controller, pick decoding, cursor refinement.

Acceptance criteria:
- [x] 500M-point scene above 30 fps: RTX 3070 Ti 47 fps (overview) / 65 fps (interior); Intel UHD 770 51 / 40 fps with the adaptive budget at 2.5–3.2M points. Node selection costs 0.9–1.0 ms on average and 2.3 ms at most per frame over 28,243 nodes. **A real mid-range card is still unmeasured (Blocked).**
- [x] Picked-point coordinates match the source file within 1 mm: tested to better than 1 µm on a georeferenced LAS about 500 km / 4,400 km from the origin, and the UI shows coordinates within 0.5 mm.
- [x] Every cleanup is undoable and audit-logged: tests plus a live check in the app (lasso, box, outliers, voxel; Ctrl+Z / Ctrl+Y).

Full results, timings and limits: `docs/phase2-report.md`. The 500M-point import took 6 minutes end to end with a 550 MB peak working set.

What was built: point streaming with source record numbers in locus-io; locus-octree (out-of-core builder, node server, region queries, scene layer with poses and removed sets, cleanup operations); measurement math in locus-analysis; schema 2 (settings, octrees, measurements, cleanup_ops); background build queue, the `locus://` node protocol and pick/measure/cleanup commands; the viewer (LOD, adaptive budget, upload budget, EDL, colour modes, clip box and plane, GPU picking, measurement and cleanup tools, About dialog); `locus-validate gen-scene` and `import`. Tests: 71 Rust, 19 frontend.

### Phase 3: Registration (done 2026-09-23; CI green on 22e9744)

Plan (step 1 comes first: every accuracy criterion is measured against it):

1. **Ground-truth scenes (`locus-synth`). Done.** Move the Phase 2 scene generator into a library crate and extend it:
   - full 6-DOF poses: yaw anywhere, plus a levelling error in roll and pitch (default up to 0.5°) and station height varying from 1.2 to 1.8 m;
   - sphere targets on stands (radius 72.5 mm) and planar checkerboard targets on walls and pillars (2 × 2 squares, black and white in intensity and colour), each ray-cast exactly;
   - optional outliers (mixed pixels along the beam) as a fraction of points;
   - fault injection for "bad link" tests: a target moved between two scans by a given offset;
   - the pose stored in the E57 is the true pose, none (identity) or perturbed by a given amount;
   - a JSON sidecar with the ground truth (true poses, target centres, radius and board normals, injected faults), and the same data from an in-memory API that registration tests call without writing files.

   Tests: every generated point, mapped through its scan's true pose, lies on a scene surface within the noise; target fits on noiseless data recover the true centres; same seed gives identical output; the sidecar round-trips.
2. **Registration core (`locus-register`, pure math, tested).** *Built: rigid fit, sphere and checkerboard detection, target matching, point-to-plane ICP, coarse alignment of levelled scans (constrained by rough poses when present), survey control and hybrid mode via the pose graph, the end-to-end pipeline (`pipeline::register`).* Rigid least-squares fit with covariance (Horn/Kabsch); sphere detection (RANSAC plus least-squares fit, radius check); checkerboard detection (planar patch plus intensity corner); target correspondence across scans by consistent distance geometry; cloud-to-cloud with FPFH features and RANSAC for the coarse pose, then point-to-plane ICP; survey control points; hybrid mode that mixes all three in one adjustment.
3. **Pose graph.** *Built: one adjustment over target, cloud and control links as point pairs (hybrid mode), χ² link test at 99.9 %, worst-first set-aside, untested lone links.* Links between scans (target, cloud or control) with covariances, then global optimisation over all scans. Each link is tested against the solution: a link whose residual is statistically inconsistent is flagged red, with the test and threshold shown.
4. **Storage and audit.** *Built: schema 3 (registrations, registration_poses, both immutable; applied registration in settings), step-by-step migration, scene uses the applied poses.* Schema 3: registrations, links, per-scan poses, detected targets; every run and every manual link change (delete, force) audit-logged. Applying a registration only updates scan poses (octrees stay as built).
5. **Graph view.** *Built: Scans → Register Scans… (Ctrl+R): run settings, stored registrations, top-view graph coloured by link status with unverified scans ringed, link figures, delete/force and re-solve, apply/revert.* Scans as nodes, links as edges coloured by error; delete or force a link and re-optimise.
6. **Registration report.** *Built: Typst PDF (`locus-report`, Go fonts), exported from the Register dialog, audit-logged with its SHA-256; bundled third-party data listed in THIRD_PARTY_NOTICES.txt (Help → About, shipped with the app, checked in CI).*
7. **Method notes** in `docs/methods/registration.md`. *Written.*

Acceptance criteria:
- [x] On synthetic scans with known poses, registration error is under 2 mm and 0.02° (relative to the first scan): 6 scans × 4M points, targets + cloud links from rough poses (0.5 m, 5° off), worst error 0.22 mm and 0.0008° (`crates/locus-register/tests/accept.rs`).
- [x] Bad links injected by the test are flagged: a 2 cm / 0.1° wrong cloud link fails the χ² test (252 per dof against 1.89) and is set aside, with accuracy kept. Without rough poses or shared targets, scans placed by shape alone are reported unverified (a flipped scan in the symmetric synthetic room is caught this way, not by χ²). *Red in the graph view once the UI exists.*
- [x] Report numbers match the internal computation: the report is rebuilt from the stored links and poses, and tests check the laid-out PDF text against the adjustment's figures for every link, the target residuals and the scan poses.

### Phase 4: 2D diagramming (approved 2026-09-23; the physical print check is under Blocked)

Plan (the hand-measurement solver first: one acceptance criterion depends on it, and diagrams are built from its output):

1. **Hand-measurement solver (`locus-analysis`, pure, tested).** Point positions from field measurements, each with a propagated uncertainty:
   - baseline/offset: a distance along a baseline between two known points, then a perpendicular offset, left or right;
   - triangulation (trilateration): distances from two or more known points; with two, the side of the baseline is stated; with more, least squares with residuals, and a check that flags an inconsistent tape;
   - measurements chain: solved points can be the known points of later measurements.
   Tests: known positions reproduced exactly from exact distances (acceptance), hand-checked values, uncertainty propagation against Monte Carlo, degenerate inputs refused (collinear, circles that don't meet).
2. **Diagram model and storage.** A diagram is a document in project coordinates (metres, SI): layers, entities (line, arc, polyline, dimension, text, symbol, north arrow, scale bar, legend, underlay), each with a stable id. Schema 4 stores each saved state as an immutable revision (full document + SHA-256), with an audit entry per revision; the editor autosaves after changes settle, with undo/redo in the editor. Measurement records keep the raw field entries and the solved positions, so a point can be traced back to the tape readings.
3. **Canvas editor (`app/src/diagram2d/`).** SVG canvas with pan and zoom, layers (visibility, lock, colour), snapping (endpoint, midpoint, perpendicular, grid) with a visible snap marker, and tools for lines, arcs, polylines, dimensions (length measured from the geometry), text, north arrow and scale bar. Geometry, snapping and hit-testing are pure functions with tests.
4. **Symbols, evidence markers and legend.** An original in-house symbol set in `assets/` (created for Locus, nothing copied). Evidence markers are numbered automatically, and renumbering is explicit. The legend is built from the symbols in use and updates live.
5. **Builders.** Room builder (walls with thickness from a polygon, door and window openings). Roadway builder (a centreline with lanes, lane widths, shoulders and markings, following curves).
6. **Underlays.** An aerial image scaled and rotated by calibrating two known points to a real distance, with the calibration residual shown and logged. A point-cloud slice: a top-down orthographic raster of the registered scene between two heights, at a chosen resolution, placed exactly in project coordinates.
7. **Print at scale.** Diagram to PDF at a chosen scale (1:50, 1:100, 1:200…) and paper size through the Typst report pipeline, with the scale bar and scale statement. Test: a 10 m line at 1:100 is 100 mm in the PDF's drawing coordinates, and the scale bar agrees.
8. **Method notes** (`docs/methods/hand-measurements.md`, `docs/methods/diagrams.md`).

Acceptance criteria:
- [x] Triangulation input reproduces known positions exactly: to 1e-12 m in `locus-analysis` tests (two and three references, both sides, baseline extension), and to 4e-16 m through the app's Measured point tool, with the tape readings stored with the point.
- [ ] A diagram printed at 1:100 measures correctly on paper. *In the PDF: proven by test on the laid-out page (a 10 m line is 100.000 mm, the 5 m scale bar 50 mm, A4 landscape exactly 297 × 210 mm), and the app prints a saved revision with its hash in the title block and the audit log. The physical check with a ruler is under Blocked, to be done before release.*
- [x] The legend updates live: it is derived from the document on every render (`legendItems`, tested), and in the app it follows added symbols and markers and drops a deleted symbol at once.

Plan status (2026-09-22): items 1–8 are built. The room and road builders store their parameters with the geometry built from them (`builders.ts`, tested: wall faces, openings, door swing, lane offsets, 3 m / 9 m broken lines, tangent-arc curves), and the printer draws that stored geometry. Underlays: an evidence image calibrated from two or more known points by a closed-form least-squares similarity, with residuals shown and the calibration audit-logged (two points are flagged as giving no check); a point-cloud slice binned on the project grid, written as a hashed PNG and audit-logged. Both are hash-checked whenever shown or printed. Checked in the app: a room with a door, a road, a calibrated aerial image (50.00 mm/px from two points) and a slice, printed to PDF. Method note: `docs/methods/diagrams.md`.

### Phase 5: 3D scene builder and asset library (approved 2026-09-23)

Plan (the scene model and extrusion first: the alignment criterion is about them):

1. **Scene document and storage.** A 3D scene is a document like a diagram: objects (extrusion of a diagram, roof, library model, light), each with a stable id, a 4×4 transform to the project frame in f64, its parameters and a material; plus the sun settings (place, date, time, time zone). Schema 5 stores immutable revisions with their SHA-256 and an audit entry each, the same way as diagrams. An extrusion names the diagram revision (id and hash) it was built from, so the 3D can always be traced to the 2D it came from.
2. **Extrusion (pure, tested).** Diagram rooms become walls of a given height built from the same outline, thickness and openings the 2D uses (doors to their head height, windows between sill and head), with an optional floor slab. Roads become lane and shoulder surfaces with the markings as thin strips just above them. Other lines can become low strips. Plan x, y are the diagram's own coordinates, exactly; z is a base elevation the examiner types or picks on the point cloud. Test: every extruded vertex lies on the diagram geometry to 1e-9 m (acceptance: within 1 mm), including after the move to the render origin.
3. **Roof builder (pure, tested).** A flat roof on any room outline; shed, gable and hip roofs on rectangular outlines, with pitch and overhang. Hip roofs on arbitrary polygons need a straight skeleton; that is deferred and logged unless a case needs it.
4. **Asset library (original, parametric).** Every model is generated in code from its dimensions, nothing imported or copied: vehicles by class (car, SUV, pickup, van, box truck, bus, motorcycle, bicycle) with editable length, width, height and wheelbase; people with height and a pose (a simple jointed skeleton with standing, sitting, kneeling and lying presets and editable joint angles); furniture (table, chair, sofa, bed, cabinet, shelving, desk); generic, unbranded weapons (handgun, long gun, knife, blunt object); numbered evidence markers matching the diagram's numbering. Tests: each model's bounding box matches its parameters.
5. **Placement and snapping.** Place models with the transform gizmo or typed values. Snap to a point cloud surface: a pick is resolved in Rust to a point, and the local surface there is fitted from its neighbours (plane fit, normal, RMS residual, point count). The model's base sits on that surface, optionally aligned to its normal, and the fit residual is shown. Tests: on a synthetic floor and a tilted plane, the snapped base lies on the plane within 1 mm and the normal within 0.5°.
6. **Materials and lighting.** Physically based material presets (colour, roughness, metalness, opacity) per object. Ambient, point, spot and directional lights, several at once, with shadows. A time-of-day sun: solar position from place and time with the NOAA algorithm, in `locus-analysis` as a pure function tested against published values, with its stated accuracy. Meshes and point clouds share one depth buffer, so they hide each other correctly with eye-dome lighting on.
7. **One frame.** Diagrams, models and point clouds use the project frame; transforms stay f64 and become origin-relative f32 only at the GPU.
8. **Method note** (`docs/methods/scene3d.md`): extrusion, roofs, surface snapping, sun position.

Acceptance criteria:
- [x] A 2D diagram and its 3D extrusion align to within 1 mm: the extrusion is built by the plan's own geometry functions, and tests put every wall vertex on the plan's lines to 1e-9 m (an irregular room with a door and a window, in a frame with coordinates near 431,200 / 5,390,110 m), road and shoulder edges on the plan's road lines to 1e-9 m, markings centred on their lines to 0.1 mm, and the f32 render-origin round trip within 1 µm.
- [x] Models can be snapped to point cloud surfaces: a plane fitted to the cloud's points around the pick (tests: within 1 mm of the true plane and 0.5° of its normal, on a floor and a tilted plane). In the app, a car snapped onto the synthetic room's floor from a fit to 35 points (RMS 0.9 mm), stored with the model in the scene revision.

Plan status (2026-09-23): items 1–8 are built. Scenes are stored as revisions (schema 5). The asset library (vehicles by class, posable people, furniture, generic weapons, evidence tents) is generated from dimensions and tested against them. Roofs: flat on any outline; shed, gable and hip on rectangles. The model gizmo moves and turns placed models. Materials use presets, and there can be several lights. The NOAA sun is tested against the NREL SPA example and casts shadows. Checked in the app: a diagram's room and road extruded, a roof added, and a car snapped to the cloud. Method note: `docs/methods/scene3d.md`.

Known limitations (logged at approval, 2026-09-23):
- **Pitched roofs need rectangular rooms.** Shed, gable and hip roofs are built only on four-cornered rooms square within 0.5°; any other outline gets a flat roof. Hip roofs on arbitrary outlines need a straight skeleton.
- **Vehicle dimensions must be editable to real values before Phase 7.** Today a vehicle has length, width, height and wheelbase over a generic class profile. Crash reconstruction needs wheelbase, front and rear track, front and rear overhang, overall length and width that match a real vehicle's specification, with the model built from them (wheels placed by track and overhang, not by a class profile). To do at the start of Phase 7.

### Phase 6: Crime analysis tools (done 2026-09-23)

Plan (ground truth first, then the tools in order of complexity; the trajectory tool sets the analysis and report pattern the other two follow, and is reviewed before they are built):

1. **Ground-truth generators.** Code in `locus-synth` (library, as the scan generator is), commands in `locus-validate`, each writing a truth file beside its data:
   - `gen-trajectory`: a bullet path from a known line through several surfaces (panels of given thickness at chosen angles), with entry and exit defects, picking noise, an optional probe rod, and an E57 of the panels with the holes.
   - `gen-bloodstain`: impacts from a known origin onto floor and walls, each stain's true impact and directional angles, and its measured ellipse with realistic noise on width, length and orientation; straight-line or gravity-affected (to show the straight-line method's bias); stain images at a known scale.
   - `gen-camera`: images rendered from known cameras (pose, focal length, principal point, radial and tangential distortion) of a scene with known control points and standing people of known heights, plus the scene as a point cloud.
2. **Analysis framework (with the trajectory tool).** Schema 6 stores each analysis run as an immutable record: tool, method version, inputs (picked points re-resolved from stored data), parameters, results with uncertainties, assumptions and limitations, and the run it revises. Every run is audit-logged. Each tool has a PDF report (Typst, as registration) with the same sections: inputs, method, results with uncertainty, assumptions, limitations.
3. **Bullet trajectory (pure math in `locus-analysis`, tested against the generator).** A line from defect centres on one or more surfaces (least squares with per-point uncertainty) or from a probe rod; azimuth and elevation in the project frame and angles to each impacted surface; an uncertainty cone (the fit's 95 % cone, and a configurable default of ±5°); possible shooter positions where the cone passes through a height band. UI in the 3D view; method note `docs/methods/trajectory.md`.
4. **Bloodstain area of origin.** Stain photos aligned to the cloud by three point pairs (or two plus the surface plane) with a transparency slider; ellipse fit from edge points and adjustable automatic edge detection; impact angle asin(width/length) and direction from the major axis and marked tail; least-squares point closest to all stain rays with per-stain residuals, excluded stains and a bootstrap 95 % ellipsoid; the straight-line warning and the upward-moving restriction (override logged with a reason). Method note `docs/methods/bloodstain.md`.
5. **Camera matching, height and witness perspective.** Before height analysis: person models defined as floor to top of head, standing, without footwear, with proportions scaled from published anthropometric data, cited in `docs/methods`. Camera solve (PnP with distortion) from 6 or more image-to-scan pairs, with the photo overlaid on the scene; subject height by reverse projection with a posable person, with uncertainty from the camera solve; witness perspective at a stated eye height with line-of-sight tests. Method note `docs/methods/camera-height.md`.

Item 5 in detail (2026-09-23):
- **Person model.** The library's person follows Drillis and Contini's proportions (stature fractions, as the camera generator), cited in code and in `docs/methods/camera-height.md`; standing, it is exactly its stature from soles to the top of the head. Done.
- **Camera solve (`locus-analysis::camera`, pure).** Image-to-scan pairs (at least 6; each scan point resolved again from stored data). Start: the normalised DLT on the pairs (needs points off one plane), decomposed into K, R, C. Refine: Levenberg–Marquardt on reprojection error in σ units (pick σ in pixels, plus each scan point's σ projected), over the pose and a lens model the examiner chooses: focal length only; + k1; + k1, k2; or full (focal length, principal point, k1–k3, p1, p2; OpenCV's model, as the generator). Covariance from JᵀJ, inflated by the Birge ratio when the residuals scatter more than stated. Reported: pose (position, heading, pitch, roll) and intrinsics with 1σ, each pair's residual, RMS, χ². Warnings for too few pairs, pairs on one plane, parameters the pairs don't pin down, a residual over 3σ.
- **Subject height (reverse projection).** The feet point (on the floor between the feet) and the top of the head clicked in the image: the feet ray meets the floor plane; the height is where the head ray passes the vertical through that point. Uncertainty by Monte Carlo over the camera's covariance and the pick σ, with a 95 % interval, and the head ray's horizontal miss from the vertical (lean, or a wrong feet point). A posable person model of that height stands at the feet point, drawn over the photo and in the 3D view; the examiner can adjust its height to match the frame, and the head point then follows the model (stated in the report).
- **Photo over the 3D scene.** The viewer takes the solved camera: pose, focal length and principal point as its projection, with the photo drawn behind the scene through the lens model (each screen pixel looked up at its distorted position), with an opacity slider. In the photo editor, scan points and the pairs' residuals are drawn over the photo too.
- **Witness perspective.** A stated eye position (a floor point and an eye height) and view direction; the 3D view shows what is visible from there, and a line-of-sight test to any picked target (scan points within a set radius of the sight line, away from its ends) says clear or blocked, and where. Stored as its own record with a report.
- **Reports** as the other tools: result, pairs table with residuals and scan sources, heights, witness lines, method, assumptions, limitations, references, sign-off. Method note `docs/methods/camera-height.md`.
- **Validation** (`locus-validate`, against `gen-camera`): pose and focal length from the generator's picked markers for the CCTV (strong distortion) and the handheld photo; heights within the spec's 2 cm with a good solve, and the stated uncertainty's coverage over seeds; in the app, one camera and one height end to end.

Acceptance criteria:
- [x] Synthetic multi-surface trajectory recovered within 0.5°: worst 0.066° over 50 generated scenes (three panels, 2 mm picking noise), the stated 95 % cone covering the truth in 94.5 % of 400 runs; in the app, picking the generated scene's defects gave 0.105°.
- [x] Area of origin mean error under 10 cm on clean synthetic data: from the generator's stain photos (alignment, automatic edges, marked tails) 0.6 mm mean over 8 rooms, the truth inside the 95 % region in all 8; in the app, six stains gave 11.5 mm. With hand-measurement noise, 233 mm with the region covering the truth 95 % of the time.
- [x] Height under 2 cm with a good camera solve (approved reading: 95 % of errors under 2 cm when the solve states the height to 1 cm, and the stated 95 % interval covering the truth about 95 % of the time over all cases): 95 % of errors under 13 mm (398 of 480 generated cases; mean 6.4 mm over all), coverage 94.6 % with few control points and 94.6 % with many; in the app, three heights on the CCTV frame within 5 mm.
- [x] All crime tools pass the `locus-validate` bounds (release, heavy tests included, 2026-09-23; Phase 13 re-checks them and regenerates the report on release):

  | Tool | Result | Bound |
  |---|---|---|
  | Trajectory, 3 panels, 2 mm picking noise, 50 scenes | worst 0.066°; 95 % cone covers the truth in 94.5 % of 400 runs; hole-centre fits 0.008° from the truth, 6/6 cross-checks agreeing | 0.5° |
  | Bloodstain, from the generator's photos, 8 rooms | mean 0.6 mm, worst 1.0 mm; region covers 8/8 (conventional point 4.8 mm, 0/8) | 10 cm mean |
  | Bloodstain, hand-measurement noise, 40 rooms | mean 233 mm; region covers 95 % (conventional 302 mm, 92 %) | coverage ≈ 95 % |
  | Bloodstain under gravity (straight-line bias shown) | origin 1.7–2.4 m too high | shown and warned |
  | Camera and height, 480 cases | above | above |

Status (2026-09-23): items 1–4 done and approved. Trajectory review decisions applied (hole-centre fits with the ellipse cross-check, angle conventions, both zones, report additions). Bloodstain review decisions applied: the conventional ray-distance point shown beside the angle fit with the validation comparison and literature, the near-round flag, an optional plan-view convergence of floor stains (separate 2-D result), and four-point perspective correction from the scale's corners. The blank close-ups were a real bug in the eye-dome pass (anything nearer than 1 m drawn as background), now fixed, with a close-up check in the viewer smoke test. Item 5 built and approved: camera matching (lens model by leave-one-out, pooled parametric bootstrap, planar start), height by reverse projection with several frames reported as a range, the photo over the 3D scene through the lens, witness perspective with line-of-sight tests, reports. Phase 6 done.

### Phase 7: Crash reconstruction tools

Plan (the Phase 6 pattern: pure, tested math in `locus-analysis` with uncertainty, commands that resolve every pick from stored data, an audit-logged analysis record, a PDF report with method, assumptions and limitations, and a method note with references):

1. **Vehicle dimensions (logged at the end of Phase 5).** A vehicle's specification fields: overall length, width and height, wheelbase, front and rear track, front overhang (rear overhang follows: length − wheelbase − front overhang), tyre diameter, and optional mass and centre-of-gravity height for the momentum and energy tools. The model is built from them: wheels placed by track and overhang, not by a class profile. The class presets become starting values. Older scenes load with values derived from their class profile, so nothing stored changes.
2. **Speed from skid marks.** `v = √(2 μ g d)` with drag factor adjusted for grade (`f = μ ± G` for a grade G, or μ cos θ ± sin θ) and braking efficiency (the fraction of the drag factor the braked wheels provide); several surfaces combined `√(v₁² + v₂² + …)`; mark lengths measured on the cloud (polyline along the mark) or entered. Each input with a range; the result with a Monte Carlo interval and the input ranges' extremes.
3. **Critical speed from yaw marks.** Radius from chord and middle ordinate `R = C²/(8M) + M/2`, and directly by a least-squares circle fit to points picked along the mark on the cloud (with its uncertainty); `v = √(μ g R)`, optionally with the superelevation (`v = √(g R (μ + e)/(1 − μ e))`).
4. **Linear momentum (2-D), two vehicles.** Masses, approach and departure angles, departure speeds (from the post-impact skid or roll-out), solved for the two impact speeds; the conditioning (approach angles too close to each other) warned about; a sensitivity table across the input ranges, and a Monte Carlo interval.
5. **Crush energy (Campbell / CRASH3).** A and B from the bundled NHTSA-derived table (done 2026-09-23, `docs/methods/crash-stiffness.md`) or user-entered with source; the crush profile from measured depths (2, 4 or 6 points) or from the point cloud (against the undamaged outline); energy by the CRASH3 integral and the equivalent barrier speed. A bundled coefficient table built from public NHTSA crash-test data is a separate item: it needs the data downloaded (permission first) and its licence and notices (rule 7).
6. **Volumetric crush comparison (Analyst Plus).** Register a damaged vehicle's scan to an undamaged reference (a second scan or the parametric model), signed deviation map and crush volume, with registration uncertainty.
7. **EDR.** No proprietary formats. CSV import and a guided entry form mirroring the pre-crash data table (time, speed, throttle, brake, steering, and optional yaw rate and acceleration), stored with its source; a vehicle driven along a scene path from the data, as an animation track for Phase 9.
8. **Reports and method notes** for each tool (`docs/methods/crash-*.md`), with references.

Acceptance criteria:
- [x] Every formula matches hand-worked examples in tests (worked independently here and shown in the tests and `docs/methods/crash.md`; published textbook examples are cited, not copied): skid (level, grade, braking efficiency, two surfaces, end speed), yaw (chord and ordinate, superelevation, CG offset, fitted circle), momentum (a locked-together collision recovered exactly, delta-V), crush (uniform, triangular, force-direction factor, equivalent barrier speed). Items 6–7 still to test.
- [ ] Sensitivity tables reproduce known textbook cases: the momentum table reproduces a hand-worked case; crush energy reproduces the NHTSA CRASH3 manual's sample run; the skid, yaw and momentum textbook cases are Blocked on Addison's examples.

Status (2026-09-23): items 1–5 built: vehicle specifications; skid, yaw (chord or a circle fitted to picked points), two-vehicle momentum with its sensitivity table, and CRASH3 crush energy, each with the range method and a Monte Carlo interval, a PDF report and the method note `docs/methods/crash.md`; checked in the app. Crush takes A and B from the NHTSA-derived table (850 tests, 698 vehicles; `docs/methods/crash-stiffness.md`) or entered with a source, and its width and depths entered or measured on the scan (both checked in the app). The crush integral reproduces the CRASH3 manual's sample run. Item 6 built: volumetric crush against an exemplar scan (pairs + ICP outside the damage region, cells over the face's plane, Monte Carlo over the registration and each cell; `docs/methods/crash-volume.md`), validated on synthetic vehicles (mean error 0.3 %, worst 1.9 %, interval covering 40/40) and checked in the app (7.89 L for a 7.854 L dent). Item 7 built: EDR pre-crash data from CSV or the form, distances with their range and Monte Carlo, positions along a picked path as the animation track (`docs/methods/crash-edr.md`), checked in the app (7.20 m, 5.61–8.79 m, as worked by hand). Items 1–8 are built; the Phase 7 acceptance box for textbook cases waits on the examiner's worked examples (Blocked).

### Phase 8: Photogrammetry (done 2026-09-23)

Plan (the licence review is in `docs/phase8-colmap-licence-review.txt`; COLMAP runs as a separate process, never linked):

1. **Licences first.** COLMAP is BSD-3, but its source bundles SiftGPU (non-commercial terms) and LSD (AGPL-3), and the official builds link CGAL and Qt (GPL/LGPL) and NVIDIA's CUDA runtime (proprietary). Nothing from COLMAP ships until Addison chooses (Blocked): A, the examiner installs COLMAP and Locus uses it from a configured path (recorded by version and SHA-256); B, a bundled CPU-only build with only permissive parts; C, B plus a CUDA build with SiftGPU patched out. Recommended: A now.
2. **Pure pieces in `locus-photo` (no COLMAP needed; tested):** a reader for COLMAP's text model (cameras, image poses, 3D points with their tracks and errors); the similarity transform that scales and georeferences a reconstruction, from known distances (scale only) or from 3 or more ground control points (7 degrees of freedom, Umeyama), with residuals, check points held out, and uncertainty; an EXIF/XMP reader for GPS position, its accuracy tags and the RTK tags DJI writes (stdlib only); WGS84 geodetic to local east-north-up coordinates.
3. **The sidecar.** Find COLMAP, read its build information (with or without CUDA), and run the stages as processes in `derived/photogrammetry/<run>/`: feature extraction, matching (exhaustive, or sequential for video), mapping, then undistortion, PatchMatch stereo and fusion when CUDA is there. Progress is read from its output; cancel stops the process tree; each stage's command line, version and exit status go in the run's record. Without CUDA: sparse only, with a clear message.
4. **Inputs.** Photos from evidence (hashed, read-only; COLMAP reads them where they are). Video sampled at a chosen interval through Windows Media Foundation (part of the OS; no ffmpeg); each frame's time and the video's hash recorded.
5. **Scale and georeference in the app.** Known distances picked on the reconstruction (with their measured values and tolerance), GCPs as picked points with surveyed coordinates, or the photos' GPS/RTK positions against the camera centres; residuals and check-point errors in the report.
6. **Into the project.** The scaled point cloud is written as a new E57 in the project, with its own hash and a provenance record: the input files' hashes, COLMAP's version and SHA-256, every stage's parameters, and the transform with its uncertainty. From then on it is a normal point cloud (registration, measurement).
7. **Report** (Typst): inputs, reconstruction statistics (registered images, mean reprojection error, track lengths), scaling and check points, provenance.
8. **Validation.** A public benchmark with ground truth (ETH3D or Strecha fountain-P11, needs download permission): scaled reconstruction measuring known distances within 1 %; missing CUDA falls back with a clear message.

Status (2026-09-23): option A chosen (examiner-installed COLMAP). Built: `locus-photo` (COLMAP runner with CPU-only features, progress, cancel and crash hints; camera models; model and PLY readers; scaling by known distances, control points or photo GPS; EXIF/RTK; ENU; Media Foundation video frames), the setup panel, runs from photos or a video, scaling by targets clicked in the photos, import as E57 evidence with an analysis record and report (`docs/methods/photogrammetry.md`). Acceptance on ETH3D pipes (laser-scan ground truth): 99th percentile 0.58 %, 100.00 % of 94,384 distances within 1 %. Missing CUDA runs sparse only with a message. In the app (ETH3D pipes photos imported as evidence; setup and run through the panel; CPU features; dense at 2000 px, 9 min on an RTX 3070 Ti): scaled by two known distances (0.8 and 0.9 mm residuals), 416 distances between 33 other ground-truth points: median 0.21 %, worst 0.45 %; 294,005 dense points imported as evidence and loaded; report printed. Review additions (2026-09-23):
- Known distances can be check-only, like control points. Each check's measured value, known value and residual is reported, and a check is flagged beyond its 95 % limit.
- Every measurement on a photogrammetric cloud takes 1σ = max(p × length, floor): p the larger of the benchmark's 0.35 % and the checks', floor the larger of the benchmark's 6 mm and the checks'. In the app: 11.564 m ± 40.8 mm.
- **Video, in the app** (ETH3D's photos as an H.264 MP4 written with Media Foundation):
  - fisheye video needed a stated field of view, which is now in the panel (COLMAP marks fisheye pairs degenerate without a focal prior);
  - 10 of 14 frames placed.
- **Control points picked on ETH3D's laser scan, in the app:**
  - picks 1–4 mm from the true points;
  - two checks at 3.4 and 3.2 mm, within 17–18 mm limits;
  - 95 ground-truth distances, worst 0.38 %.

Acceptance criteria:
- [x] On a public benchmark dataset with ground truth, scaled reconstruction measures known distances within 1 %.
  - ETH3D pipes, laser-scan ground truth: 99 % of about 95,000 distances within 1 % in every run (99th percentile 0.51–0.83 % over five runs; worst single distance 0.97–1.43 %).
  - In the app: worst 0.45 % (photos, scaled by known distances) and 0.38 % (video, scaled by control points on the scan).
- [x] Missing CUDA falls back gracefully with a clear message: sparse only, the reason in the panel and the report.

### Phase 9: Animation and cameras

Plan:

1. **Motion math, `locus-analysis::motion`, tested.** This is the single authority for positions over time. Playback samples it and reports and renders evaluate it per frame, so the acceptance test covers the real code. It covers:
   - paths through picked points: centripetal Catmull–Rom with curvature-continuing ends, or straight segments; exact arc-length parametrisation (Gauss–Legendre table, then Newton);
   - speed profiles: constant speed; phases of constant acceleration that stop at zero and never reverse; time–distance tables for EDR records and keyframes, interpolated monotonically (Fritsch–Carlson);
   - tracks: a path, a profile, a start time and an offset along the path, giving position, direction of travel, heading, distance, speed and acceleration at any time and at every frame;
   - keyframed objects (lights, props): position and heading at keyframe times, interpolated.
2. **Timeline in the scene** (`app/src/animation/`): play, pause, scrub and loop over a time ruler, with a track per animated object.
   - Vehicles and people from the scene builder follow a path picked on the cloud with a profile. An EDR run's time–distance track (Phase 7) can drive a vehicle directly.
   - A vehicle's heading follows the path's tangent at its reference point, the rear axle, set from its wheelbase.
   - The animation is stored with the scene and audit-logged like other scene edits.
3. **Camera rigs:**
   - orbit (a centre, radius, height and period);
   - fly-through (a camera path with a look-ahead or fixed targets);
   - follow (an offset in an object's frame, smoothed);
   - driver view (an eye point in a vehicle's frame);
   - mirror views (a reflected camera for a mirror's plane and size);
   - 360° panorama (a cube map rendered, then equirectangular).
4. **Time–distance–speed report** from any animation: a table per moving object at a chosen interval (time, position, distance travelled, speed, acceleration), the distances between chosen objects over time, and each object's inputs (path, profile, source such as an EDR run) and method. Typst PDF, audit-logged.
5. **Render to MP4** at a chosen resolution and frame rate:
   - frames rendered offscreen, exactly duration × fps + 1 of them, each at its timeline time;
   - encoded with Media Foundation's H.264 sink writer (as `locus-photo::video_dev`, moved into app code), with no ffmpeg;
   - after writing, the file is read back and checked (frame count and duration against the timeline);
   - on macOS and Linux, render is refused with a message, like video import.
6. **Validation:**
   - an object moving at a set speed is at the right position at every frame (a unit test, done);
   - rendered frame count and duration match the timeline (tested by reading the file back);
   - EDR-driven motion reproduces the record's distances.

Acceptance criteria (SPEC):
- [ ] An object moving at a set speed shows the correct position at every frame (tested).
- [ ] Rendered video frame count and duration match the timeline.

Status (2026-09-23): item 1 done. `motion` has the paths, profiles and tracks. `animation` has the movers with sourced segments, the time zero, lighting, driver and witness views (60° default, wider warned), the plausibility checks (friction bands, speed and heading jumps), the assumed-segment list and the render log type; all tested. The requirements from the Phase 9 review are written up in `docs/methods/animation.md`.

Item 2 built (`app/src/animation/`), stored in the scene document so every edit is a scene revision:
- time zero (event and basis, flagged until both are given), range and lighting;
- movers linked to scene models, paths picked on the cloud (vehicles by the rear axle), each segment and the path and friction with a source; an EDR record adds its time–distance table, its speeds and its own path in one step;
- the timeline: ruler, playhead, play, pause and scrub; measured segments solid, assumed hatched; flags marked; a live speed and distance readout; "Needs attention" and the assumed list;
- in the view: models follow their movers, others show as markers, and each path is drawn blue (measured), orange (assumed) or red (flagged).

Found and fixed in the app: an EDR table interpolated from distances alone rippled the acceleration to 1.8× the record's (0.72 against 0.40 m/s²), which would raise false friction flags; the record's speeds now set the slopes, giving its exact per-interval deceleration. The heading-jump check flagged a tight smooth curve at every step; it now needs a step well above its neighbours (a real corner). Checked in the app: an EDR-driven car at −1.00 s is at 6.20 m, 1.20 m/s, and its model 1.35 m ahead of the axle as its wheelbase puts it.

Item 3, first part (2026-09-23):
- Views: driver (eye in the vehicle's frame; a typical seat by default, recorded as an assumption), witness (at a picked point, looking at a point or tracking a mover), orbit and follow.
- Every view's camera is computed with the motion (`animation::camera`, tested), and played back with "look through".
- Human views default to 60° and warn when wider. Presentation cameras are labelled as nobody's point of view.
- The driver's own vehicle isn't drawn from the seat, and the report carries that as a limitation.
- Checked in the app: the driver's eye at −1.00 s is 1.215 m ahead of the axle (45 % of the 2.7 m wheelbase), 0.35 m left and 1.2 m up, at 60°.

Items 4 and 5 (2026-09-23):
- **Time–distance–speed report** (`locus-analysis::tds`, `locus-report::animation`; 1aa2f34):
  - tables per mover, with source ranges (range method) and "Assumed" rows;
  - pair distances, with closing speed as an option;
  - time and distance to picked points;
  - checked in the app: at −1.00 s the EDR car is at 6.20 m (5.82–6.58), 1.20 m/s (0.92–1.48).
- **MP4 render** (`locus-photo::mp4`, `src-tauri/src/render_cmds.rs`, `app/src/animation/RenderSection.tsx`), checked in the app:
  - the driver view, every overlay, 1280×720 at 30 fps over −6 to 1 s: 211 frames, read back 211 frames over 7.033 s (211/30), in 10 s;
  - logged as analysis 3 with its SHA-256;
  - every frame carries the driver-view label; the overlays, the scale bar with its depth, and "Illustrative" while Mover 2's assumed motion runs were all checked on extracted frames.
- **Fixed from that check:** a segment's speed read 0 at its first instant (`Profile::at` treated t = 0 as before the start).

Acceptance so far:
- [x] An object moving at a set speed shows the correct position at every frame (tested: `an_object_at_a_set_speed_is_right_at_every_frame`).
- [x] The rendered video's frame count and duration match the timeline: checked by reading every render back, and in the app (211 frames, 7.033 s).

Next: the fly-through, mirror and 360° cameras (item 3's rest), then close Phase 9.

## Blocked

- **Physical print-scale check at 1:100** (2026-09-22): print a scaled diagram PDF at actual size and measure it (the 10 m line and the 100 mm calibration bar in the title block) with a ruler. The PDF geometry is proven by tests; the physical check must be done before release.

- **Mid-range GPU measurement** (2026-09-22): Phase 2's frame-rate criterion has been measured on an RTX 3070 Ti and an Intel UHD 770 only. It still needs a run on a real mid-range card (e.g. RTX 3060 or GTX 1660).

- **Textbook cases for skid, yaw and momentum** (2026-09-23): Addison will supply worked examples (inputs and published answers) from a reconstruction textbook, to be cited, not copied; until then the tests use hand-worked examples. Crush energy is already checked against the NHTSA CRASH3 manual's sample run.

- **Video import on macOS and Linux** (2026-09-23): frames are sampled with Windows Media Foundation; on other systems video import is refused with a message. Needs a permissive decoder (the OS's own: AVFoundation on macOS; on Linux, nothing permissive and complete; ffmpeg is LGPL/GPL) before Phase 14's other platforms.

- **Bundle identifier** (2026-09-22): `app.locus.desktop` is a placeholder. Addison will supply the real publisher domain before Phase 14; it must change before the first signed release.
