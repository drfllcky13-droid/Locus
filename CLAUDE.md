# Locus: forensic scene reconstruction software

Locus is a desktop application for documenting, analyzing, and presenting crime and crash scenes from 3D laser scans, drone imagery, and hand measurements. It is an original, clean-room product. The full build plan is in `docs/SPEC.md`. Read it before starting any phase. Track progress in `docs/PROGRESS.md`.

## Non-negotiable rules

1. **Clean room.** Never copy code, UI assets, icons, symbol libraries, file formats, names, or text from FARO Zone, FARO SCENE, or any other commercial product. Implement features from public forensic science and open standards only. Do not name any competitor in code, UI, or docs shipped with the app.
2. **Evidence integrity.** Source evidence files are read-only forever. Hash every imported file (SHA-256) and store the hash in the project. Every operation that changes analysis state is appended to the project audit log. Never silently alter measured data; derived data is always stored separately from source data.
3. **Units.** All internal geometry is SI (meters, radians, seconds, kg) in `f64`. Convert only at the UI and report boundary. Every numeric UI field shows its unit.
4. **Forensic math is pure and tested.** All analysis math lives in `crates/locus-analysis` as pure functions with no I/O or UI dependency. Every function has unit tests with hand-verified values plus property tests. Every result carries an uncertainty or residual, never a bare number.
5. **Explain the method.** Each analysis tool records which method, inputs, assumptions, and limitations were used, so the report can show its work. An examiner must be able to defend every number in court.
6. **Tests before merge.** `cargo test`, `cargo clippy -- -D warnings`, `pnpm test`, and `pnpm typecheck` must pass before a phase is marked done.
7. **Licensing.** Dependencies must be MIT, Apache-2.0, BSD, Zlib, MPL-2.0, or Unicode-3.0. Ask before adding anything GPL, AGPL, LGPL, or with unclear terms. Data-file exception: CC-BY-4.0, CC-BY-SA-3.0, LPPL, the W3C licence, the permissive hyphenation-pattern licences (FSF all-permissive, and the custom Bulgarian and Sanskrit pattern licences) and the Sublime HQ packages licence are allowed only for data files bundled unmodified or converted to another format without changing their content (hyphenation patterns, citation styles, character tables, syntax definitions), never for code. Slovak and Hungarian hyphenation patterns are excluded (GPL/MPL-1.1); hypher is patched to drop them. Every bundled third-party data file must have an entry in THIRD_PARTY_NOTICES.txt; CI checks this.
8. **Desktop only.** Locus ships as an installed desktop app. There is no web version and no hosted deployment, and nothing in the repo should be set up to publish it as a website.

## Stack

- Shell: Tauri 2 (Rust backend, web frontend). Primary target Windows 10/11 x64; macOS and Linux must build.
- Core: Rust workspace under `crates/`.
- UI: React + TypeScript + Vite under `app/`. 3D rendering with Three.js. 2D diagrams on HTML canvas.
- Reports: Typst, embedded, for PDF generation.
- Photogrammetry: COLMAP (BSD) invoked as an external sidecar binary.

## Working method

- Work one phase at a time, in order. Start each phase by writing a short plan in `docs/PROGRESS.md`, then build, then check every acceptance criterion in the spec.
- Keep commits small and scoped, one feature per commit.
- When a spec detail is ambiguous, pick the option that best preserves accuracy and auditability, record the decision in `docs/DECISIONS.md`, and continue.
- If a task needs something you cannot do (a paid SDK, proprietary format, hardware), stub it behind a trait, note it in `docs/PROGRESS.md` under "Blocked", and move on.

## Commands

- `pnpm tauri dev`: run the app
- `cargo test --workspace`: Rust tests
- `pnpm -C app test`: frontend tests
- `cargo run -p locus-validate`: accuracy validation suite (Phase 13)

## Planning

Claude Code plans and builds. `docs/PLAN.md` is the single, living plan: goal and scope, standing rules and working principles, decisions by area, remaining work in order, open questions for Addison, and a short status. Read it at the start of every session.

- Keep PLAN.md current: update it when a decision is made, a phase closes or the order changes. Log each decision in `docs/DECISIONS.md` as before, and summarise it in PLAN.md.
- Decide routine matters yourself. Ask Addison only about PLAN.md's open questions or what is genuinely his call (licensing, legal, lab practice, spending, hardware), and batch the questions.
- Before closing a phase, check every acceptance criterion, run it in the app, and ask how opposing counsel would attack each result.
- Replies: work without chat commentary. The only message to Addison is a short note when a phase is done and you're ready for the next one, or a question only he can answer.
