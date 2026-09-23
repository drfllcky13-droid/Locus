PROJECT: Locus
BATON → CHAT
Carry: Chat answers the three decisions below (render order, driver-view obstruction, report scope); then Code builds the time–distance–speed report and the MP4 render.
Status: Phase 9 (animation): items 1–2 done; item 3 has driver, witness, orbit and follow views. Fly-through, mirror and 360° are not started.
Blocked on: nothing for Phase 9. Standing: the examiner's textbook cases (skid, yaw, momentum), the 1:100 ruler check, the mid-range GPU run, the bundle identifier, video on macOS and Linux.

## Report for Chat

**Your latest prompt was already done.** The six requirements (sources, plausibility checks, time zero, render overlays and log, driver and witness views, low light) are in `locus-analysis::animation` and `docs/methods/animation.md` (42e95ed). The timeline UI followed in 561a60f; see the previous report in git history (0a0819f). I continued with the plan as your prompt said.

**What changed since** (9eb88b4):
- **Views.**
  - **Driver:** the eye is stated in the vehicle's frame. Until it's measured it defaults to a typical seat (45 % of the wheelbase forward of the rear axle, 0.35 m left, 1.2 m up), recorded as an assumption so the report lists it.
  - **Witness:** stands at a picked point with an eye height (1.6 m default, assumed until stated), and looks at a picked point or tracks a mover.
  - **Orbit and follow:** presentation cameras, labelled in the UI and docs as nobody's point of view. No human field-of-view warning applies to them.
- Every view's camera is computed alongside the motion (`animation::camera`, tested), so playback and renders use the same cameras. "Look through" plays any view in the 3D pane.
- Driver and witness views default to 60°, with the value shown. A wider value is warned in the view editor and in "Needs attention".
- From the driver's seat, the driver's own vehicle isn't drawn, since its interior isn't modelled. That is added to the report's limitations: no obstruction by pillars, mirrors, dashboard or tint is shown.
- Refused: a driver view of anything that isn't a vehicle, a view whose mover is missing, a field of view outside 1–179°, and an orbit with no radius or period.

**Tests:**
- Rust: clippy clean; locus-analysis 89 passed. The new `cameras_follow_their_movers` test hand-checks the driver, witness, orbit and follow cameras.
- Frontend: 103 passed.
- In the app (`scratch/locus/anim_ui.mjs`): looking through the default driver view at −1.00 s puts the eye at 7.912, 1.360, 1.200. That is the axle at 6.697 plus 1.215 forward (45 % of 2.7 m), 0.35 m left and 1.2 m up, with a 60.00° horizontal field of view. The assumed list shows the default eye. No page errors.

**Decisions needed:**
1. **Order of the remaining rigs.** (a) Build fly-through, mirror and 360° now. (b) Build the time–distance–speed report and the MP4 render first, then those three rigs. Recommend (b): the report and render are the acceptance items, and the three rigs are presentation only.
2. **Obstruction in the driver's view.** (a) Leave it as it is, with the limitation stated. (b) Add simple A-pillar and mirror silhouettes from entered dimensions, stated as approximate. Recommend (a) for now: a wrong pillar is worse than a stated absence.
3. **Scope of the time–distance–speed report.** (a) One table per mover at a chosen interval, plus distances between chosen pairs of movers over time. (b) Also a closing-speed column for each pair. Recommend (a), with (b) as an option, since closing speed follows directly from the pair distances.
