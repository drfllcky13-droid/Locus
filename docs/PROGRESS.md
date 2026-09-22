# Progress

Current phase: **Phase 0 (in progress)**

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

Local setup done: rustup stable (1.98.1, MSVC) and VS 2022 Build Tools (C++ workload) installed via winget.

## Blocked

- **GitHub repo for CI** (2026-09-22): the GitHub token available to Claude can't create repositories (403). Needs an empty private repo `locus` created by Addison; then `git remote add origin … && git push -u origin main` and confirm the Actions run is green.
