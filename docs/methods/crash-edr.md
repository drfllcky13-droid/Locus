# EDR pre-crash data

Locus doesn't read EDR retrieval tools' files. Those formats are proprietary and are not reverse engineered (SPEC). The examiner brings the pre-crash data table in one of two ways:
- as a CSV exported or transcribed from the retrieval report;
- typed into a form laid out like that table: 5 s before the trigger at 2 samples a second, with rows added or removed as needed.

The tool gives:
- the distance travelled from each sample to the end of the record, with its uncertainty;
- given a path picked in the scene, where the vehicle was at each sample. That list is the track the Phase 9 animation timeline drives a vehicle along.

Code: `crates/locus-analysis/src/edr.rs` (tested). Command: `crash_preview` / `crash_save` with `tool: "edr"`.

## Data

**Columns are recognised by their headers:**
- time;
- speed;
- accelerator or throttle;
- brake;
- steering;
- yaw rate;
- longitudinal acceleration.

**Units** are read from brackets in a header where given, for example "Speed (mph)", "Time (ms)" or "Long. accel (g)". A speed column without a unit is read in the unit the examiner chooses.

**Format:**
- Commas, semicolons or tabs separate the fields.
- Lines starting with # are comments.
- Other columns, such as engine RPM, are listed as not used.
- Brake reads on/off, yes/no, 1/0 or applied/not applied.
- Times must increase, and speeds must be 0 or more.

**Stored in the analysis record:**
- the data as imported, or as built from the form (canonical CSV), with its SHA-256;
- how each column was read;
- the source the examiner gives (the retrieval report, the vehicle, who retrieved it). The source is required.

## Distance

The distance from sample i to the end time is the integral of speed over time. Between samples the speed lies between its two recorded values. So the integral lies between:
- the left Riemann sum (each interval at its starting speed);
- the right Riemann sum (at its ending speed).

This is exact when the speed changes monotonically between samples. The value is their mean, which is the trapezoid rule.

The speed tolerance is a scale (%) and an offset (km/h). It is systematic: every sample is off by the same factor and offset.

The default, ±1 km/h, is the recording accuracy 49 CFR Part 563 requires of the indicated speed signal. That is how faithfully the recorder stores the vehicle's own speed signal, not how close that signal is to the true speed over the ground. The examiner can widen the tolerance but not narrow it below ±1 km/h. Widening needs a reason, which is stored in the record and printed in the report. Reasons to widen include:
- wheel slip under braking;
- ABS cycling;
- wheelspin;
- non-original tyre sizes or axle ratios.

The four inputs, each a range, go through the same range method and 20,000-draw Monte Carlo as the other crash tools:
- the scale k;
- the offset b;
- the integration rule w, from left (0) to right (1);
- the braking a in any gap.

For sample i, with L and R its left and right sums to the last sample and T the time between them:

d = k [(1 − w) L + w R] + b T + gap

**The gap.** When the end time is after the last sample, the gap is covered at the last speed v, braking at a anywhere from 0 to 1 g:
- v·g − ½ a g², where g is the gap's length in seconds;
- or v²/2a if the vehicle would stop first.

**Worked in the tests:**
- 20, 20, 18, 16, 14 m/s at 0.5 s steps: left sum 37 m, right sum 34 m, trapezoid 35.5 m.
- With ±1 % and ±1 km/h the range is 34 × 0.99 − 2/3.6 to 37 × 1.01 + 2/3.6 m.
- A 0.5 s gap at 14 m/s: 7 m, or 5.77 m braking at 1 g.

## Path

The examiner picks the path in order, ending where the vehicle's reference point was at the end time. Each sample's position is its nominal distance back along the path from the end, with the direction of travel. The range method's low and high distances mark the span along the path it could be in; the report and 3D view show it in red.

Before the path's start, the vehicle continues straight back along the first segment, and the report warns.

Checked in the tests on an L-shaped path. In the app, a 7.2 m record on a 7.2 m path picked on the floor put the first sample at the path's start (0.50, 1.01 m; picked at 0.5, 1.0 m).

## Assumptions and limitations

Listed in each report. In short:
- EDR speed usually comes from wheel or transmission speed. Wheel slip (hard braking, spinning wheels), tyre size and axle ratio make it differ from speed over the ground.
- The ±1 km/h default is the recording accuracy of the indicated speed, not the accuracy of the true ground speed. The two can differ by more:
  - with wheel slip under braking;
  - under ABS cycling, where the wheels repeatedly slow below the vehicle's speed and recover;
  - with wheelspin;
  - with non-original tyre sizes, which change the wheel speed to ground speed ratio.

  Where these may apply, widen the tolerance and give the reason.
- Sample timing is uncertain. A recorder may not sample exactly at the stated times, and different signals may be sampled at different moments within an interval. The distance range does not include this.
- Sample times are relative to the recorder's trigger, not to the impact. The retrieval report gives the relation.
- The vehicle is assumed to have followed the path. The path's own measurement uncertainty is not added to the positions.
- The data isn't checked against the recorder's own files.

## References

- US Code of Federal Regulations, 49 CFR Part 563, Event Data Recorders: the required data elements, their recording intervals and accuracy.
- SAE J1698, Vehicle Event Data Interface: output data definitions.
