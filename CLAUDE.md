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

## Handoff protocol
Chat plans, Code builds, the user relays. HANDOFF.md is the mailbox.

Start of every session and every scheduled check-in: read HANDOFF.md.

Do it yourself, no handoff: bug fixes, red CI, refactors, dependency bumps, and anything already specified in the current phase plan.
Hand to CHAT: end of a phase or numbered item, a design decision, unclear or conflicting requirements, or stuck after two attempts.
Hand to ME only for things only the user can do: merges, approvals, credentials, real-device or real-world checks.

End of every task: overwrite HANDOFF.md with the footer below plus a "## Report for Chat" section (what changed, commit hashes, test results, decisions needed with options). Commit and push it with the work. End your reply with the footer:

PROJECT: <name>
BATON → CODE / CHAT / ME / IDLE
Carry: <the single next action and who does it>
Status: <phase + what's done, one line>
Blocked on: <what needs the user, or "nothing">
