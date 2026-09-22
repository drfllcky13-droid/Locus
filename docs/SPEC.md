# Locus build specification

## 1. Product goal

Locus takes an investigator from raw field data to a court-ready deliverable in one application: import scans and photos, register them into one accurate scene, diagram it in 2D and 3D, run crime and crash analyses with stated uncertainty, animate events, and export reports, videos, and a portable offline viewer.

Three license tiers, built as feature flags on one codebase:

| Tier | Includes |
|---|---|
| Diagram | 2D diagramming, hand measurements, symbol library, reports |
| Analyst | Everything in Diagram, plus 3D scenes, point clouds, all analysis tools, photogrammetry, animation, portable viewer, VR |
| Analyst Plus | Everything in Analyst, plus in-app scan registration and volumetric crush comparison |

## 2. Architecture

```
locus/
  CLAUDE.md
  docs/            SPEC.md, PROGRESS.md, DECISIONS.md, methods/ (one page per analysis method)
  crates/
    locus-core       project model, IDs, units, coordinate frames, audit log
    locus-io         importers/exporters (E57, LAS/LAZ, PLY, PTS/XYZ, OBJ/glTF, images, CSV)
    locus-octree     out-of-core octree builder and chunk server for point clouds
    locus-register   target detection, ICP, global registration, pose graph, reports
    locus-analysis   pure forensic math (bloodstain, trajectory, height, crash)
    locus-photo      COLMAP sidecar orchestration, camera models, scaling/georeferencing
    locus-report     Typst templates and PDF generation
    locus-validate   synthetic ground-truth scenes and accuracy test suite
  src-tauri/       Tauri shell; thin command layer that calls crates
  app/             React + TypeScript UI
    src/viewer3d/    Three.js scene, point cloud LOD renderer, gizmos, measurement
    src/diagram2d/   canvas diagram editor
    src/tools/       one folder per analysis tool UI
    src/animation/   timeline, keyframes, camera rigs
  viewer/          stripped-down Tauri build for the portable offline viewer
  assets/          original symbol and 3D model library (created in-house only)
```

Data flow: importers write source data untouched into the project and produce derived octree chunks. The UI never touches raw files; it requests chunks and calls typed Tauri commands. All mutations go through `locus-core` so they are audit-logged.

## 3. Project file format (`.locus`)

A folder bundle (zipped on export) containing:
- `project.sqlite`: entities, transforms, measurements, analyses, animations, audit log (append-only table, each row hash-chained to the previous row).
- `evidence/`: original imported files, byte-for-byte, with SHA-256 recorded at import.
- `derived/`: octree chunks, thumbnails, photogrammetry outputs. Always regenerable from evidence.
- `assets/`: user-added models and images.

Coordinate frames: one project frame (right-handed, Z up, meters). Every scan and model stores a 4x4 transform to the project frame. Optional georeference stored as EPSG code plus offset, to keep `f32` GPU precision safe by rendering relative to a local origin.

## 4. Phases

Each phase lists what to build and acceptance criteria. A phase is done only when every criterion passes.

### Phase 0: Scaffold and CI
Build: Cargo workspace with all crates as stubs, Tauri 2 app with React/Vite/TS, lint and format configs, GitHub Actions running tests on Windows and Linux, `docs/PROGRESS.md` and `docs/DECISIONS.md`.
Accept: `pnpm tauri dev` opens a window with an empty 3D viewport and a menu bar. CI is green.

### Phase 1: Project model, evidence integrity, import
Build: `.locus` create/open/save; audit log with hash chain; importers for E57 (including multiple scans with poses and embedded panoramas), LAS/LAZ, PLY, PTS, XYZ, OBJ, glTF; JPEG/PNG with EXIF. Import dialog shows file hash and point count.
Accept: Importing a file never modifies it (test compares hashes before and after). Tampering with the audit log is detected on open. A 100M-point E57 imports without exceeding 4 GB RAM.
Note: FARO native `.fls` files require a vendor SDK license. Do not reverse engineer them. Users export E57 from their scanner software; document this in the import dialog help text.

### Phase 2: Point cloud engine
Build: out-of-core octree (Potree-style, own format) built in a background task with progress; LOD streaming renderer in Three.js with eye-dome lighting, RGB/intensity/elevation coloring, point size by LOD; clipping boxes and planes; point picking with snapping; distance, angle, area, height-above-plane measurements; cleanup tools (box delete, lasso delete, statistical outlier removal, voxel downsample) that write derived layers, never touching source.
Accept: 500M-point scene stays above 30 fps on a mid-range GPU. Picked-point coordinates match the source file within 1 mm. Every cleanup is undoable and audit-logged.

### Phase 3: Registration (Analyst Plus)
Build: automatic detection of spheres (RANSAC fit, radius check) and checkerboard targets; target-based registration (least-squares rigid fit); cloud-to-cloud (FPFH features + RANSAC coarse, point-to-plane ICP fine); survey control points; hybrid mode mixing all three; pose graph optimization across all scans; an interactive graph view showing scans as nodes and links as edges, colored by error, where the user can delete or force links; registration report (per-link error, overlap %, max/mean target residuals, overall statistics) exported to PDF.
Accept: On synthetic scans from `locus-validate` with known poses, registration error is under 2 mm and 0.02 degrees. Bad links injected by the test are flagged red. Report numbers match the internal computation.

### Phase 4: 2D diagramming (all tiers)
Build: CAD-style canvas with layers, snapping (endpoint, midpoint, perpendicular, grid), lines, arcs, polylines, dimensions, text, north arrow, scale bar, legend auto-built from used symbols; measurement entry from hand-measurement methods (baseline/offset, triangulation) that solves point positions; roadway and room builders; evidence markers with auto-numbering; aerial image underlay with scale calibration from two known points; import a top-down orthographic slice from a point cloud as an underlay.
Accept: Triangulation input reproduces known positions exactly. Printed diagram at 1:100 measures correctly on paper. Legend updates live.

### Phase 5: 3D scene builder and asset library
Build: extrude 2D diagrams into 3D (walls with height, doors, windows, roads with lanes); place 3D models with gizmos; roof builder; PBR materials; lighting including multiple light sources and time-of-day sun; an original in-house library of generic vehicles (sized by class and editable dimensions), people (posable, height-adjustable), furniture, weapons (generic, non-branded), and evidence markers. Diagram, models, and point clouds share one coordinate frame.
Accept: A 2D diagram and its 3D extrusion align to within 1 mm. Models can be snapped to point cloud surfaces.

### Phase 6: Crime analysis tools (Analyst)
Write `docs/methods/<tool>.md` for each, covering formulas, assumptions, limitations, and literature references.

Bloodstain area of origin:
- Import scaled stain photos; align each to the point cloud by 3 point pairs (or 2 points plus surface plane); transparency slider to verify.
- Fit an ellipse to each stain (manual edge points plus auto edge detection the user can adjust).
- Impact angle `alpha = asin(width / length)`; directionality from the major-axis orientation and the user-marked tail direction.
- Each stain yields a 3D ray on its surface. Area of origin is the least-squares point minimizing summed squared distance to all rays. Report per-stain residuals, excluded stains, and a 95% confidence ellipsoid from bootstrap resampling.
- State in the UI and report that straight-line methods ignore gravity and drag and tend to overestimate height; restrict to stains the analyst marks as upward-moving unless overridden with a logged justification.

Bullet trajectory:
- Define a trajectory from defect points on one or more surfaces, or from a probe/rod line picked in the scan.
- Compute horizontal (azimuth) and vertical angles relative to project frame and to each impacted surface.
- Draw an uncertainty cone (default plus/minus 5 degrees, configurable) and extend to find possible shooter positions within a height band.
Accept: Synthetic multi-surface trajectory recovered within 0.5 degrees.

Height and perspective analysis:
- Camera matching: solve camera pose and intrinsics (PnP with lens distortion) from 6+ image-to-scan point pairs; overlay the photo or CCTV frame on the 3D scene.
- Subject height by reverse projection: place a posable person model, adjust until it matches the frame; report height with uncertainty from the camera solve.
- Witness perspective: place a camera at a stated eye height and position and render what was visible, with a line-of-sight test to any target.

Accept (all crime tools): pass the `locus-validate` error bounds in Phase 13.

### Phase 7: Crash reconstruction tools (Analyst)
Methods docs required, as in Phase 6.
- Speed from skid: `v = sqrt(2 * mu * g * d)`, with drag factor adjustments for grade and braking efficiency; combined speed for multiple surfaces `sqrt(v1^2 + v2^2 + ...)`.
- Critical speed from yaw marks: radius from chord and middle ordinate `R = C^2 / (8M) + M / 2`, then `v = sqrt(mu * g * R)`; also fit a circle directly to points picked on the mark in the scan.
- Linear momentum (2D) for two-vehicle collisions with approach and departure angles; solve for impact speeds; sensitivity table across input ranges.
- Crush energy using the Campbell/CRASH3 stiffness model (A and B coefficients, user-entered or looked up from a bundled table built from public NHTSA crash test data); crush profile from measured depths or from the point cloud.
- Volumetric crush comparison (Analyst Plus): register a damaged vehicle scan to an undamaged reference model, compute signed deviation map and crush volume.
- EDR data: the major retrieval tools use proprietary formats. Do not reverse engineer them. Build a CSV import plus a guided entry form that mirrors the pre-crash data table (speed, throttle, brake, steering at time steps), then drive a vehicle along the scene path from that data.
Accept: every formula matches hand-worked examples in tests; sensitivity tables reproduce known textbook cases.

### Phase 8: Photogrammetry (Analyst)
Build: orchestrate COLMAP as a sidecar (feature extraction, matching, sparse and dense reconstruction) with a progress UI and cancel; read EXIF GPS and RTK tags for georeferencing; scale by known distance or by GCPs; accept video input by sampling frames at a chosen interval; output goes into the project as a normal point cloud with its own hash and provenance record.
Accept: on a public benchmark dataset with ground truth, scaled reconstruction measures known distances within 1%. Missing CUDA falls back gracefully with a clear message.

### Phase 9: Animation and cameras (Analyst)
Build: timeline with keyframes for any object, camera, or light; vehicle paths along splines with speed profiles (constant, acceleration, from EDR); people along walk paths; camera rigs (orbit, fly-through, follow, driver view, mirror views, 360 panoramic); time/distance/speed report generated from any animation; render to MP4 at chosen resolution and frame rate.
Accept: an object moving at a set speed shows the correct position at every frame (tested). Rendered video frame count and duration match the timeline.

### Phase 10: Reports and exports
Build: Typst report templates (scene summary, evidence list with hashes, registration report, each analysis with method, inputs, results, uncertainty, and limitations, diagrams at scale, figures); exports to PDF, PNG/TIFF at chosen DPI, DXF for diagrams, E57/LAS for point clouds, glTF for scenes, CSV for measurements.
Accept: a report generated twice from the same project is byte-identical apart from the timestamp. Every number in a report traces to an audit log entry.

### Phase 11: Portable offline viewer
Build: "Export case package" writes a folder (fits on a USB drive) with the viewer executable plus selected scene data, diagrams, photos, reports, and animations. The viewer is read-only: navigate, measure, play animations, view panoramas, open reports. No install, no internet.
Accept: package opens on a clean Windows machine without admin rights. Viewer cannot modify data, and shows the package hash on its About screen.

### Phase 12: VR (Analyst)
Build: WebXR session from the 3D viewer (works with SteamVR/OpenXR headsets via the webview, or document a fallback if the webview blocks WebXR); teleport and smooth movement; measure with controllers; inspect evidence markers; show animations at 1:1 scale.
Accept: stable 72+ fps on a supported headset with a 100M-point scene.

### Phase 13: Validation suite (critical)
Build `locus-validate`, which generates synthetic ground-truth scenes and runs every tool end to end:
- Registration: known poses, added noise and outliers.
- Bloodstain: simulated impacts from known origins onto walls and floors with realistic ellipse noise.
- Trajectory: known lines through multiple surfaces.
- Camera match and height: rendered images from known cameras with known subject heights.
- Crash: textbook scenarios with known answers.
Outputs a validation report (PDF) listing each tool's mean error, max error, and standard deviation across hundreds of runs.
Accept: area of origin mean error under 10 cm on clean synthetic data; trajectory under 0.5 degrees; height under 2 cm with a good camera solve; registration under 2 mm. The report is regenerated in CI on every release.
Also: write a protocol in `docs/methods/validation-protocol.md` for physical validation studies (staged scenes, multiple blind examiners), since synthetic tests alone will not satisfy a court.

### Phase 14: Polish, onboarding, licensing
Build: guided workflow panel (step-by-step for each common job: indoor crime scene, fatal crash, fire scene); contextual help per tool; sample project; license tiers as feature flags with an offline license file; crash reporting that never uploads case data; installer (MSI) and auto-update with signed builds.
Accept: a new user can complete the sample indoor crime scene workflow in under 30 minutes following the in-app guide.

## 5. Known hard problems (decide early, log in DECISIONS.md)

- GPU precision: render relative to a local origin; never send georeferenced coordinates to the GPU as `f32`.
- Memory: octree building must stream; never load a full scan into RAM.
- WebView limits: test WebXR and large buffers in the Tauri webview early (Phase 2), and switch to a native wgpu renderer for the point cloud if performance fails.
- Proprietary formats (native scanner files, EDR tools, evidence-management systems): support through vendor-exported open formats or official partner programs only.
