# Progress

Current phase: **Phase 1 (complete locally; awaiting final CI run)**. Phase 0 complete.

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

### Phase 1: Project model, evidence integrity, import

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

Open issues:
- OBJ files are read fully into memory (`tobj`); this is acceptable for meshes and is marked with a `ponytail:` comment.
- LAS files that name only an EPSG code make the examiner choose the unit; resolving it needs an EPSG table.
- The `e57` simple-reader bug should be reported upstream.

## Blocked

- **Bundle identifier** (2026-09-22): `app.locus.desktop` is a placeholder. Addison will supply the real publisher domain before Phase 14; it must change before the first signed release.
