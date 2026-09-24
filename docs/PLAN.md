# Lotus plan

The single, living plan for Lotus. It says what Lotus is for, the rules it is built under, the decisions taken and why, what is left in what order, and what only Addison can answer. The detail lives elsewhere and is linked, not copied:
- the build specification: [SPEC.md](SPEC.md);
- the phase log and the Blocked list: [PROGRESS.md](PROGRESS.md);
- every decision with its full reasoning, dated: [DECISIONS.md](DECISIONS.md);
- one note per method: [methods/](methods/);
- the rules: [../CLAUDE.md](../CLAUDE.md).

Claude Code does both the planning and the building from 2026-09-23. Before that a separate planner ("Chat") made the decisions and reviewed each phase; that role ended with the plan-handover prompt of 2026-09-23. This file is updated whenever a decision is made, a phase closes or the order changes.

## Status

*Updated 2026-09-24.*

- **Done.** Phases 0–11, 13 and 14 are done. Phase 11's clean-machine check is under Open questions. Phase 7's textbook-case criterion is blocked (see Open questions).
- **Phase 9 (animation), closed 2026-09-23.** It covers:
  - the motion model with provenance, time zero and plausibility checks;
  - the timeline UI;
  - driver, witness, orbit, follow, fly-through, mirror and 360° views;
  - the time–distance–speed report;
  - MP4 render with read-back checks.
- **Phase 10 (reports and exports), closed 2026-09-24.** Reports are reproducible and traceable. There is a case report, and diagrams export as PNG, TIFF and DXF, point clouds as E57, LAS and LAZ, the scene as glTF, and measurements as CSV, all hashed and logged ([exports.md](methods/exports.md)).
- **Phase 11 (portable case package), closed 2026-09-24.** A folder with the program itself as a read-only viewer, the case data, freshly printed reports and the renders, all hashed in a manifest whose hash is the package hash ([case-package.md](methods/case-package.md)).
- **Phase 13 (validation), closed 2026-09-24.** `locus-validate run` prints the validation report for every release ([validation.md](methods/validation.md)), and the physical study protocol is written ([validation-protocol.md](methods/validation-protocol.md)). The full run found the hand-measured bloodstain region under-covering (81 %); bloodstain method version 2 corrects the bias (coverage now 97 %).
- **Phase 14 (polish), closed 2026-09-24.** Offline signed licences with Diagram, Analyst and Analyst Plus tiers, crash reports that stay on the machine with paths removed, a "?" beside each tool opening its method note, guides for indoor, crash and fire scenes that tick themselves, and a generated sample case ([licensing.md](methods/licensing.md)). The signed MSI, updates and the 30-minute new-user test wait on Addison.
- **Installer and updates, 2026-09-24:** MSI built and checked; a `v*` tag publishes a release with the MSI, update files and validation report; the in-app updater is in. The repository is public; releases wait only on the signing secret.
- **Next:** every planned phase is built. What remains is Addison's list and the Blocked items.
- **Waiting on Addison:** his to-do list is [ADDISON-TODO.md](ADDISON-TODO.md) (also summarised under Open questions).

## 1. Goal and scope

- **What Lotus is.** An original, clean-room desktop application with the capabilities of the top commercial forensic scene packages. It covers 2D and 3D documentation, analysis and court presentation of crime and crash scenes. Addison, the owner, works in law-enforcement forensic services.
- **The overriding goal.** Crime and crash tools must produce results an examiner can defend in court. Every other design choice gives way to it.
- **Registration is in scope, but only so far.** The top tier of such packages registers scans itself, so Lotus does too. Lotus is **not** a full scan-processing suite: registration exists only to get scan data into the analysis tools.
- **Desktop only.** Lotus is an installed app, with no web version and no hosted deployment, and the repo is never published as a website. Network features (auto-update, crash reporting that never uploads case data) **are** allowed. An earlier offline-only rule was superseded on 2026-09-22; both entries are kept in DECISIONS.md.
- **No VR.** Phase 12 (VR) was dropped by Addison on 2026-09-23.
- **Three licence tiers**, as feature flags on one codebase: Diagram, Analyst, Analyst Plus. See [SPEC.md §1](SPEC.md).

## 2. Standing rules and constraints

The rules in [CLAUDE.md](../CLAUDE.md), in short:
1. **Clean room.** Nothing is copied from any commercial product; features come from public forensic science and open standards; no competitor is named in anything shipped.
2. **Evidence integrity.** Evidence is read-only and SHA-256 hashed; every change is in the hash-chained audit log; derived data is kept apart from source data.
3. **Units.** SI in `f64` inside; conversion only at the UI and report boundary; every field shows its unit.
4. **Forensic math is pure and tested,** in `locus-analysis`, with hand-verified and property tests. Every result carries an uncertainty or a residual.
5. **Explain the method.** Each run records its method, inputs, assumptions and limitations, so the report shows its work.
6. **Tests before merge:** cargo test, clippy with warnings as errors, pnpm test and typecheck.
7. **Licensing.** Dependencies must be MIT, Apache-2.0, BSD, Zlib, MPL-2.0 or Unicode-3.0; ask before anything else.
   - Data-file exception: some other licences are allowed for data files bundled unmodified or format-converted, never for code.
   - Every bundled data file is in THIRD_PARTY_NOTICES.txt, and CI checks it.
8. **Desktop only** (above).

Working principles, established in review:
- **Honest uncertainty.** Every result carries an uncertainty, and validation shows its stated 95 % interval covers the truth about 95 % of the time, poor inputs included. Deliberately conservative ranges are acceptable when they are stated as such.
- **Show the conventional method.** Where Lotus uses a better but less conventional method, it also computes and reports the conventional result, with the reason and the validation comparison: admissibility weighs general acceptance.
- **Label judgment against computation.** Examiner-set values (the ±5° trajectory zone, assumed motion, a default seat position) never look like computed or measured results.
- **Refuse rather than degrade.** Examples: a print scale that doesn't fit is refused rather than shrunk; a pick beyond the encoding limits is refused; a changed underlay image is refused.
- **Saved records never change silently.** When an algorithm changes, saved measurements keep their stored values; a revision is a new record.
- **Ground truth before tools.** The synthetic truth is built in `locus-synth` / `locus-validate` first, then the tool.
- **Every phase closes with a real run in the app,** not only unit tests; those runs keep finding bugs that tests missed. Before closing a phase, check every acceptance criterion and ask how opposing counsel would attack each result.
- **Proprietary formats only through open exports.** Scanner files, EDR tool files and evidence-management systems are reached only through vendor-exported open formats or official partner programs, never by reverse engineering.
- **CI is kept lean** (free GitHub Actions minutes):
  - Linux on every push and pull request;
  - Windows nightly, skipped when nothing changed in 25 hours;
  - every OS and the heavy tests on release tags and on runs by hand;
  - a pre-push hook runs fmt and clippy.

## 3. Decisions and reasons

A summary by area. The dated entries with full reasons and alternatives are in [DECISIONS.md](DECISIONS.md), and the method detail is in the linked notes.

**Platform and rendering**
- **Stack:** Tauri 2 + Rust + React/TypeScript/Three.js. It gives fast native math and file handling, quick UI work and one codebase.
- **Rendering** stays in the WebView on WebGL2, since the Phase 2 spike passed all four criteria ([spike-rendering.md](spike-rendering.md)). WebGPU is the upgrade path.
- **The viewport renders on demand,** not in a loop, and redraws when chunks load.
- **Octrees** are built per scan in scan-local metres, and every point lives in one node. The GPU draws relative to a local origin.
- **Precision never comes from the GPU.**
  - A pick only identifies the point; coordinates resolve in Rust from f64 data.
  - The pick-encoding limits are a hard guard: a pick beyond them is refused.
  - Picks prefer the nearest surface, and saved measurements keep their stored coordinates ([measurement.md](methods/measurement.md)).
- **GPU selection:** NVIDIA and AMD high-performance exports plus the WebView2 flag; the active GPU is shown in Help → About.
- **Lasso delete** defaults to "visible surface only", with "all depths" as an option and a count preview ([cleanup.md](methods/cleanup.md)).

**Project and evidence**
- **Saving:** no Save button. Each change is its own SQLite transaction with its audit entry, and the chain head is mirrored and checked on open ([audit-log.md](methods/audit-log.md)).
- **Evidence** is re-hashed on every project open.
- **Import** is preview then commit. The unit is required when a format declares none, and duplicate hashes are refused.
- **3D scenes and diagrams** are immutable revisions. Analyses are one immutable record type, withdrawn with a reason, never deleted.

**Licences and reports**
- **Licence decisions:**
  - Unicode-3.0 is approved.
  - The data-file exception covers unmodified or format-converted data.
  - Slovak (GPL) and Hungarian (MPL-1.1) hyphenation are dropped by a local hypher patch, and CI fails if they return.
  - First-party data uses LicenseRef-Lotus-Proprietary, and the notices check has no unexplained ignores.
  - Crates are UNLICENSED with publish = false.
- **Reports use Typst,** compiled in-process and sealed: one template, one data file, no file access, no clock. It was chosen over a custom generator to avoid throwaway work. All analysis reports share one layout.

**Diagrams** ([diagrams.md](methods/diagrams.md))
- The road broken-line pattern is configurable (3 m / 9 m by default).
- Every scaled print carries a 100 mm calibration bar.
- A two-point underlay calibration warns that it has no fit check.

**Registration** ([registration.md](methods/registration.md))
- Targets: sphere centres come from a fixed-radius fit; checkerboard precision is bounded by the point spacing.
- Coarse alignment assumes levelled scans (FPFH features with level consensus) and uses the scanner's rough pose when there is one.
- ICP covariance is formal and stated as a lower bound.

**3D scene** ([scene3d.md](methods/scene3d.md))
- Library models are generated in code, so there is nothing to copy or license.
- Snapping fits a plane to the cloud.
- The sun position uses the NOAA algorithm.
- Pitched roofs are built for rectangular rooms only.

**Trajectory** ([trajectory.md](methods/trajectory.md))
- The hole centre comes from a fit to the hole's rim ellipse. The ellipse's width over its length is compared with sin(impact angle) as a cross-check.
- Angles are reported both surface-relative and scene-relative; the convention is a setting.
- The two cones are labelled "Measurement uncertainty (95 %, computed)" and "Examiner-defined zone (analyst judgment)".
- The report has plan and elevation figures, the picked points, the source scan and revision, case and examiner fields, sign-off and defect photos.

**Bloodstain** ([bloodstain.md](methods/bloodstain.md))
- The fit in angle space is the primary origin, with the conventional ray-distance point alongside (validation coverage 8/8 against 0/8).
- Wall stains need a clearly upward direction (beyond 2σ).
- Floor stains give a separate 2D convergence in plan, on by default (the lab uses it; Addison, 2026-09-24).
- Four-point perspective correction warns when the correction is large.
- Near-round stains are flagged when they dominate.

**Camera and height** ([camera-height.md](methods/camera-height.md))
- The uncertainty is a bootstrap pooled across lens models that can't be told apart (coverage 94.6 %).
- The bound: 95 % of errors under 2 cm when 1σ ≤ 1 cm.
- The lens model is chosen by leave-one-out; a planar scene starts with stated assumptions.
- Eye height is examiner-stated only, since no validated ratio exists.
- The person model follows Drillis and Contini (via Winter 2009).
- Gait, footwear, headwear and posture are stated limitations; several frames give a range.

**Crash** ([crash.md](methods/crash.md), [crash-stiffness.md](methods/crash-stiffness.md), [crash-volume.md](methods/crash-volume.md), [crash-edr.md](methods/crash-edr.md))
- Inputs are ranges; results are given as the value, the range-method extremes and a Monte Carlo 95 % interval.
- Stiffness A and B come from public NHTSA barrier tests (Campbell's method), with test IDs, uncertainty, single-test flags and an examiner override.
- Crush is checked against the CRASH3 technical manual.
- The crush-volume range is deliberately conservative (40/40 coverage).
- The mirrored-opposite-side reference carries a symmetry uncertainty and an asymmetry warning; it reads about 5 % high.
- EDR's ±1 km/h is the minimum tolerance: it is the 49 CFR 563 recording accuracy, not ground speed. Widening needs a logged reason, and wheel slip, ABS, tyres and timing are stated limitations.

**Photogrammetry** ([photogrammetry.md](methods/photogrammetry.md))
- The examiner installs COLMAP (option A). Bundling would break rule 7 because of SiftGPU's non-commercial terms, AGPL LSD, GPL/LGPL CGAL and Qt, and the proprietary CUDA runtime.
- Its version and executable hash are recorded, and a CPU-only setting avoids SiftGPU. Nothing is added to the notices.
- OpenMVG (MPL-2.0) is logged as a possible future bundled sparse pipeline.
- Benchmark data stays outside the repo.
- Video goes through Windows Media Foundation, not ffmpeg.
- Distances and points can be check-only.
- Per-measurement uncertainty is max(percentage term, absolute floor).
- Acceptance: 99 % of distances within 1 %.

**Animation** ([animation.md](methods/animation.md))
- Each segment has a source (EDR, analysis, measured, or assumed), and so do paths, friction and views.
- Plausibility checks flag accelerations beyond (μ ± tolerance) g (friction circle), speed jumps and corners in a path.
- Every animation defines its time zero.
- EDR speeds set the timing curve's slopes.
- Driver and witness views default to 60°, printed, with wider warned. Orbit and follow are labelled nobody's point of view.
- Night scenes make no visibility claim from renders.
- Render overlays are off by default.
- Renders are read back, checked and logged with their SHA-256.
- The time–distance–speed report and the render came before the extra cameras.
- Every driver-view render carries "Vehicle interior (pillars, mirrors, dashboard) not shown".

## 4. Remaining work, in recommended order

1. ~~**Phase 13, validation.**~~ Done.
   - Consolidate the per-tool validation into one `locus-validate` run and PDF, regenerated for every release.
   - Write `docs/methods/validation-protocol.md` for physical studies (staged scenes, several blind examiners), since synthetic tests alone won't satisfy a court.
2. **Phase 14, polish.**
   - Guided workflows (indoor crime scene, fatal crash, fire scene), contextual help and a sample project.
   - Tier licensing.
   - A signed MSI installer, auto-update, and crash reporting that never uploads case data.

**Backlog** (unordered; promote when justified):
- an exemplar vehicle's interior scan attached to the driver's vehicle frame, so view obstructions come from measured data;
- a bundled OpenMVG pipeline;
- parallel outlier removal;
- pitched roofs on non-rectangular rooms;
- video import and render on macOS and Linux;
- bloodstain runs from floor stains only;
- a WebGPU renderer;
- Evidence.com integration through Axon's partner program (Addison's decision).

## 5. Open questions for Addison

Answered 2026-09-24 (DECISIONS): no licence needed, in-house use, name Lotus, identifier `io.github.drfllcky13-droid.lotus`, releases on GitHub Releases, WiX approved, no angle convention (both printed), floor convergence used (on by default), no Evidence.com, CI stays free, his unit runs the validation studies. Still open, on [ADDISON-TODO.md](ADDISON-TODO.md):
- **Textbook worked examples** from Fricke (Northwestern): the inputs and published answers for 3–5 skid, yaw and momentum examples.
- **The physical print check:** measure a 1:100 print with a ruler.
- **A case package on a clean machine,** as a standard user, from a USB drive.
- **A mid-range GPU run,** for example on an RTX 3060 or a GTX 1660.
- **The new-user test:** the indoor guide on the sample case in under 30 minutes.
- **Physical validation studies:** when.
- **A recorded conflict, resolved by the repo.** The handover prompt described the heavy tests as running per push on Linux. The CI (2026-09-23) runs them only on release tags and on runs by hand, to save minutes. The repo's newer record stands; say if you want them per push.
