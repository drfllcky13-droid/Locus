# Volumetric crush against a reference scan

The volume tool measures how much of a vehicle's outer surface was pushed in. It compares the damaged vehicle's scan with an undamaged reference: a scan of an exemplar vehicle of the same make, model and body.

- Code:
  - `crates/locus-register/src/exemplar.rs`: the registration.
  - `crates/locus-analysis/src/crush_volume.rs`: the volume and its uncertainty. Both are tested.
- Command: `crash_preview` / `crash_save` with `tool: "volume"`, in `src-tauri/src/crash_cmds.rs`.
- Generator: `locus-validate gen-crush`.

## Inputs

- The damaged vehicle's scan and the reference scan, both imported into the project.
- At least 3 picked pairs: the same undamaged feature on the reference, then on the damaged vehicle, away from the damage.
- The damage region: the clip box, set around the damage.
- The cell size (20 mm by default).

## Registration

1. **Rigid fit to the pairs** (Arun et al. 1987) gives the starting pose. The report lists each pair with its residual after the final registration.
2. **Point-to-plane ICP** (Besl & McKay 1992; Chen & Medioni 1992) refines it, using the locus-register implementation with Huber weights:
   - the damaged scan's points within 1 m of the pairs and the region, but outside the region, are the target, with normals facing that scan's scanner;
   - the reference, thinned to 20 mm, is the source.

   Leaving the region out keeps the damage from pulling the fit.
3. **Uncertainty.** The ICP's covariance treats every pair of points as independent, so it is too small: neighbouring points share their errors. It is scaled by the number of pairs per 10 cm patch (about 20 on typical data), as if each patch were one independent observation. The report prints the factor and the resulting 1σ in translation and rotation.

## Mirrored opposite side as the reference

Without an exemplar, the reference can be the damaged vehicle's own undamaged side, reflected. Code: `locus-register::exemplar::{centre_plane, align_mirror}`.

1. **The centre plane.** The examiner picks at least 3 pairs of symmetric features (mirror bases, lamp corners, wheel centres), each on the left and then its counterpart on the right, all on the damaged scan.
   - The plane's normal is the pairs' mean direction.
   - It passes through the centroid of their midpoints.
   - The report lists how far each midpoint lies off the plane and each pair's angle to the normal, and warns above 20 mm or 5°.
2. **The reference.** The vehicle's points outside the damage region are reflected across the plane. The damaged points are left out before reflecting, so the mirrored damage never enters. The reflection is then registered by ICP onto the undamaged surfaces, as an exemplar is, starting from each pair's reflected point landing on its counterpart.
3. **Symmetry uncertainty.** Real vehicles aren't exactly symmetric: manufacturing tolerance, earlier repairs, damage elsewhere, load and suspension. Where both sides are undamaged, each reflected point's offset from the original along the original's normal is:
   - averaged per 5 cm patch;
   - combined as an RMS over the patches;
   - reduced by the part the points' noise leaves in a patch mean.

   The result is the symmetry σ. In each Monte Carlo draw, one offset drawn with that σ is added to the whole reference surface. The model is systematic: a side standing proud reads as crush over the whole region.
4. **The report** always warns that the reference is mirrored and that asymmetry inside the damage region can't be measured. It adds that as a limitation, and warns when the symmetry σ exceeds 5 mm.

**Validation.** `mirrored_crush_volume_*` in `crates/locus-validate/tests/crush_volume.rs` uses:
- a symmetric vehicle front, 1.6 m wide, with a dent right of centre;
- the left half standing 0–2 mm proud, as real asymmetry;
- three symmetric pairs picked with 3 mm error;
- a random pose.

Over 40 runs:
- mean error +5.0 %, worst +16 %, biased upward by the asymmetry;
- the 95 % interval covered the truth in 39 of 40.

In the app (`gen-crush`'s `symmetric.e57`, 1 mm asymmetry, true volume 7.854 L):
- result 8.16 L, 95 % 7.60–8.98 L;
- symmetry σ 0.8 mm.

## Volume

1. **The plane.** Inside the region, a plane is fitted to the reference's points. Its normal points outward, toward the damaged scan's scanner.
2. **The cells.** The plane is divided into square cells. In each cell, each surface's height above the plane is the median of its points, and needs at least 3 points.
   - The crush depth is the reference's height minus the damaged's, positive inward.
   - Its 1σ combines the two medians' uncertainties: 1.2533 × the points' spread / √n, with the spread at least the scan's point σ.
   - Cells where the reference has a surface but the damaged scan doesn't are counted as uncovered and left out.
3. **Crushed or pushed out.** A cell counts as crushed when its eight neighbours' mean depth is positive. The crush volume is the sum of those cells' signed depths × cell area. The remaining cells make up the volume pushed outward.

   The class comes from the neighbours, not from the cell's own depth, so the cell's own noise can't decide which sum it joins. Noise therefore averages out in both sums. Summing max(depth, 0) over every cell would add about 0.4σ × cell area for each undamaged cell: +2 to +8 % in the first validation run, with the interval covering the truth 0/20 times.
4. **Monte Carlo** (200 draws, seeded). Each draw:
   - moves the reference by a draw from the scaled registration covariance and rebuilds the cells;
   - draws each cell's depth from a normal about its value with its 1σ;
   - sums the crushed volume.

   The report gives the 95 % interval, the mean and the 1σ.

The report also gives:
- the deepest cell;
- the area deeper than 3σ;
- a depth map of the cells: red for crushed inward, blue for pushed outward, white where unchanged.

In the 3D view, the cells deeper than 2σ are drawn on the face's plane.

## Validation

`crates/locus-validate/tests/crush_volume.rs` generates the synthetic vehicles:
- A vehicle's front corner: the front, a side and the bonnet, 1.6 × 1 × 1 m, sampled every 8 mm.
- Range noise of 1–3 mm.
- A paraboloid dent in the front, with radius 0.15–0.30 m and depth 30–150 mm. Its exact volume is π R² D / 2.
- The damaged copy placed at a random pose.
- Five feature pairs picked with 3 mm 1σ error.

It then registers and measures as the app does.

- 8 runs in the default suite; 40 in the heavy one.
- Result: mean error 0.3 %, worst 1.9 %.
- The 95 % interval covers the truth in 40 of 40 runs, so on this data, whose noise is independent, it is conservative. With the unscaled (formal) ICP covariance it covers 39 of 40.

**The interval is deliberately conservative.** The registration covariance is scaled as if each 10 cm patch were one independent observation, because real scans' errors are correlated: neighbouring points share range bias, incidence effects and registration error. On synthetic vehicles with independent noise, where no such correlation exists, the interval covered the true volume in 40 of 40 runs; a calibrated 95 % interval would miss about 2. The report says so in its method section.

In the app, on `gen-crush`'s scans (dent 0.25 × 0.08 m, 7.854 L):
- four pairs picked on the cloud;
- the clip box set around the dent;
- result 7.89 L, 95 % 7.76–8.02 L;
- ICP 2.2 mm RMS, 100 % overlap.

## Warnings

The report warns when:
- more than 10 % of the cells are uncovered;
- the ICP didn't converge;
- the overlap is under 50 %;
- the pairs fit worse than 10 mm RMS;
- the geometry barely constrains one direction (conditioning under 0.01);
- no net inward crush was found.

## Assumptions and limitations

Listed in each report. In short:
- The reference must match the damaged vehicle outside the damage.
- The damaged face must be a height field over its plane: each line along the normal meets each surface once. A face that wraps around a corner should be measured in parts.
- Torn-open areas can read deeper than the surviving skin, because the damaged scan sees into the vehicle there.
- Uncovered cells are left out, so occlusion understates the volume.
- Differences between the two vehicles (load, tyre pressure, ride height, trim) are not in the uncertainty.
- The volume is not converted to energy.

## References

- K. S. Arun, T. S. Huang and S. D. Blostein, "Least-squares fitting of two 3-D point sets", IEEE Transactions on Pattern Analysis and Machine Intelligence 9(5), 1987.
- P. J. Besl and N. D. McKay, "A method for registration of 3-D shapes", IEEE Transactions on Pattern Analysis and Machine Intelligence 14(2), 1992.
- Y. Chen and G. Medioni, "Object modelling by registration of multiple range images", Image and Vision Computing 10(3), 1992.
