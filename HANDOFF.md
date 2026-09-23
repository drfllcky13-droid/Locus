PROJECT: Locus
BATON → CHAT
Carry: Chat reviews the Phase 9 timeline UI (561a60f) and gives any decisions on camera rigs and driver/witness views; then Code builds item 3.
Status: Phase 9 (animation): items 1–2 done (motion model with provenance, time zero and plausibility checks; timeline UI, checked in the app).
Blocked on: nothing for Phase 9. Standing: the examiner's textbook cases (skid, yaw, momentum), the 1:100 ruler check, the mid-range GPU run, the bundle identifier, video on macOS and Linux.

## Report for Chat

**What changed** (561a60f, on top of 42e95ed):
- Timeline UI in `app/src/animation/`, saved in the scene document, so every edit is an audit-logged scene revision.
  - Time zero (event and basis; flagged until both are given), range and lighting.
  - Movers linked to scene models, on paths picked on the cloud (a vehicle's path is its rear axle's).
  - Each segment, path and friction value carries a source: an EDR record, another analysis, evidence, or an assumption with its reason.
  - An EDR record adds its time–distance table, its speeds and its own path in one step.
  - The timeline: ruler, playhead, play, pause and scrub. Measured segments are solid and assumed ones hatched; flags are marked. It has a live speed and distance readout, "Needs attention", and the list of assumed segments.
  - In the 3D view, models follow their movers and anything unlinked shows as a marker. Paths are drawn blue (measured), orange (assumed) or red (flagged).
- A Tauri command, `animation_evaluate`, evaluates the animation with `locus-analysis::animation`, the same code the report and renders will use.
- Two fixes found in the app:
  - **False friction flags from EDR timing.** Interpolating from distances alone gave 0.72 m/s² where the record's deceleration is 0.40. Table segments now take optional speeds as slopes (cubic Hermite), which gives exactly the record's per-interval deceleration.
  - **False heading-jump flags.** A tight but smooth curve was flagged at every step. Now a step is flagged only when it turns well beyond the steps either side, which means a real corner.
  - Both are documented in `docs/methods/animation.md`.

**Tests:**
- Rust: fmt and clippy clean; locus-analysis 88 passed, including the new tests for Hermite slopes, corner-only heading jumps and smooth curves.
- Frontend: 102 passed, including `animation/model.test.ts`.
- In the app (`scratch/locus/anim_ui.mjs`): an EDR-driven car at −1.00 s is at 6.20 m and 1.20 m/s. Its model is 1.35 m ahead of the axle, as its wheelbase puts it. The friction flag reads 0.40 m/s², an assumed mover without a reason is warned, playback and scrub work, the scene saved, and there were no page errors.

**Decisions needed for item 3 (camera rigs and views):**
1. Driver eye point default. (a) Fixed defaults: 1.2 m high, 0.35 m left of centre, at the B-pillar line. (b) Required entry, with nothing shown until it's entered. Recommend (a), printed as "default" in the report until edited.
2. Which rigs first. (a) All six in item 3 (orbit, fly-through, follow, driver, mirror, 360°). (b) Driver, witness, orbit and follow now, with fly-through, mirror and 360° after render works. Recommend (b).
3. Witness target. (a) A picked point. (b) A mover, tracked over time. Recommend both, as a choice.
