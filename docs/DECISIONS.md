# Decisions

Record each design decision as: date, decision, reason, alternatives considered.

- **2026-09-22**: Stack is Tauri 2 + Rust core + React/TypeScript/Three.js UI. Reason: fast native math and file handling, quick UI iteration, one codebase for the main app and the portable viewer. Alternative: fully native C++/Qt (slower to build), Unity/Unreal (heavy, licensing).
- **2026-09-22**: No support for proprietary scanner or EDR formats without official vendor licensing. Reason: clean-room requirement and legal risk. Users import E57 and CSV instead.
- **2026-09-22**: TypeScript pinned to 6.x. Reason: typescript-eslint 8 refuses to run on TS 7. Alternative: drop type-aware linting (loses checks). Revisit when typescript-eslint supports TS 7.
- **2026-09-22**: `locus-validate` exits non-zero until it has real scenarios. Reason: an empty validation run must never read as a pass.
- **2026-09-22**: Bundle identifier `app.locus.desktop` is a placeholder until a publisher domain is chosen; it must be fixed before the first signed release (Phase 14), since changing it moves the app-data folder.
- **2026-09-22**: The 3D viewport renders on demand (camera change or resize), not in a continuous loop. Reason: idle GPU and battery use on field laptops. Animation playback (Phase 9) will run its own loop while playing.
