# Progress

Current phase: **Phase 1 (in progress)**. Phase 0 is done except the CI-green check (see Blocked); Addison approved starting Phase 1 before CI runs.

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
- [ ] CI is green (Windows + Linux): workflow written, not yet run (see Blocked)
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
- [ ] Importing a file never modifies it (test compares hashes before and after)
- [ ] Tampering with the audit log is detected on open
- [ ] A 100M-point E57 imports without exceeding 4 GB RAM

## Blocked

- **Bundle identifier** (2026-09-22): `app.locus.desktop` is a placeholder. Addison will supply the real publisher domain before Phase 14; it must change before the first signed release.
- **Unicode-3.0 license** (2026-09-22): Tauri's dependency tree (url → idna → icu_*) includes 18 crates under Unicode-3.0, a permissive license that is not on CLAUDE.md's allowlist. They can't be removed without dropping Tauri. It is allowed provisionally in `deny.toml`; needs Addison's OK.

- **GitHub repo for CI** (2026-09-22): the GitHub token available to Claude can't create repositories (403). Addison has created the empty private repo; waiting for its URL. Then `git remote add origin … && git push -u origin main` and confirm the Actions run is green.
