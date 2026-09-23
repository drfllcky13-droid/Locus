# Animation

An animation shows vehicles and people moving through the scene over time, and views from chosen eye points. It illustrates a reconstruction. It is not a new measurement: every motion comes from stated inputs, and the report says which are measured and which are assumed.

- Code: `crates/locus-analysis/src/motion.rs` (paths, profiles, tracks) and `crates/locus-analysis/src/animation.rs` (the animation, provenance, time zero, plausibility checks, views, the render log). Both are tested.
- Storage: the animation is stored with its scene, as scene revisions are, so every change is audit-logged.

## Motion

**Paths.** A mover follows a path through points picked in the scene. The path is either:
- a centripetal Catmull–Rom spline, whose ends continue the curvature of their last three points;
- straight segments.

Positions are found by exact arc length: a Gauss–Legendre table per segment, then Newton's method.
- A vehicle's path is its rear axle's centre. Its heading is the chord to the point one wheelbase further along the path, the way a vehicle's body follows a curve.
- A person's heading, and anything else's, is the path's tangent.

**Segments.** A mover's motion is a sequence of segments. Each is one of:
- constant speed;
- constant acceleration, from a stated speed or from the previous segment's end speed. Braking stops at zero and never reverses;
- a table of times and distances, such as an EDR record's (Phase 7) or keyframes, interpolated monotonically (Fritsch–Carlson). When the speeds at those times are known, as an EDR record's are, they set the curve's slopes instead (cubic Hermite). An EDR record's distances are its speeds' trapezoidal sums, so each interval is then exactly constant acceleration, (v₁ − v₀)/Δt: the plausibility checks see the record's own decelerations, not the interpolation's ripple (which reached about 1.8 times the true value on a test record).

Each segment has a duration; the last may run to the end of the timeline. Before its first segment a mover waits at its start; after its last timed segment it stays where it stopped.

**One source of positions.** Playback, the time–distance–speed report and renders all take positions from `motion`. The acceptance test checks that code directly: an object at a set speed on a curved path is at v·t along it at every frame at 30 fps (to 10⁻⁹ m), with the arc length checked against a brute-force polyline to 0.1 mm.

## Provenance of each segment

Every segment records its source:
- an **EDR record** in this project: the analysis and its name;
- **another analysis** (skid, yaw, momentum and so on);
- **measured evidence** (a scan, photo or video), with a note;
- the **examiner's assumption**, with the reason.

A mover's path and its friction value have sources too.

**Assumed segments:**
- marked in the timeline;
- listed, with their times and reasons, in the report;
- optionally labelled "Illustrative" in renders.

A path or segment without a measured source is never shown as if it were measured.

## Time zero

Every animation states its time zero:
- the event, for example "EDR trigger", "impact" or "first frame of the video";
- how that event is known.

Timeline times are seconds from it, negative before it. The report states it. Renders can overlay it, off by default.

## Plausibility checks

Evaluated at 100 Hz over the timeline. Each finding is shown in the timeline and in the report's "Needs attention" box.

**Friction.** For a mover with a stated friction μ ± tolerance (with its source), the combined acceleration is tested. That combination is √(longitudinal² + lateral²), a friction circle.
- Longitudinal acceleration comes from the segment.
- Lateral acceleration is v²κ, with the path's curvature κ measured over ±5 cm of it.
- Two bands:
  - beyond (μ + tolerance) g: "beyond what the stated friction allows";
  - between (μ − tolerance) g and (μ + tolerance) g: "at the limit of the stated friction".
- Consecutive samples in a band are merged into one interval, with its peak.
- A vehicle with no stated friction is flagged as not checked.

The curvature is the drawn path's own. A spline through points on a circle ripples a few percent in curvature about it, so peaks can exceed the circle's v²/r by that much.

**Speed jumps.** A change of more than 0.1 m/s between one segment's end and the next one's start is flagged as instantaneous.

**Heading jumps.** A turn of more than 2° within one 0.01 s step while moving, and more than three times the turn in the steps either side, is flagged. It comes from a corner in a straight-segment path. A tight but smooth curve turns about as much every step, so it isn't flagged here; the friction check covers whether it can be driven.

## Views

**Driver view.** The eye's position is stated in the vehicle's frame: forward of the rear axle, left of centre, and up from the ground.

**Witness view.** The witness stands at a picked floor point, with a stated eye height, looking toward a target.

**Field of view.**
- Both default to a 60° horizontal field of view. That is about what a person attends to looking ahead, not the full extent of human vision, about 200°.
- The value is printed with every view in the report, and on the render when overlays are on.
- A wider field of view is warned: it makes things look farther away and smaller than a person there would see them.
- Eye position, height and field of view each have a source, like segments.

## Low light

An animation states its lighting: daylight, or low light (dusk, dawn, night, or artificial light only).

For low light, the report always carries this limitation. The brightness and contrast of rendered images do not represent what a person could see: human visibility depends on adaptation, glare, headlamp patterns and contrast that a render does not reproduce. No conclusion about visibility is drawn from renders; that needs a visibility study.

## Renders

Renders are MP4 at a chosen resolution and frame rate:
- exactly (to − from) × fps + 1 frames, each at its timeline time;
- written with Windows Media Foundation (no ffmpeg);
- read back after writing to check the frame count and duration.

**Overlays** are all optional and off by default:
- elapsed time (from time zero);
- frame number;
- each mover's speed;
- a scale bar;
- "Illustrative" on assumed segments;
- the time-zero event.

**Each render is logged** (in the audit log, and listed in the report) with:
- the scene and its revision;
- the audit log's head hash (the project state it came from);
- the view;
- the settings: resolution, frame rate, time range and overlays;
- the frame count;
- the file and its SHA-256.

## Assumptions and limitations

Listed in every report. In short:
- Positions between the stated inputs are interpolated.
- Real motion between those inputs is not known.
- Assumed segments are illustrative.
- In low-light scenes, renders don't represent visibility.

## References

- E. Catmull and R. Rom, "A class of local interpolating splines", in *Computer Aided Geometric Design*, Academic Press, 1974; P. J. Barry and R. N. Goldman, "A recursive evaluation algorithm for a class of Catmull–Rom splines", SIGGRAPH 1988; C. Yuksel, S. Schaefer and J. Keyser, "Parameterization and applications of Catmull–Rom curves", Computer-Aided Design 43(7), 2011: the centripetal form.
- F. N. Fritsch and R. E. Carlson, "Monotone piecewise cubic interpolation", SIAM Journal on Numerical Analysis 17(2), 1980.
