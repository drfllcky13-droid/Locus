# Bloodstain area of origin

The point in space a set of spatter stains came from, estimated from each stain's shape and direction as straight-line paths back from the stains, with its uncertainty and each stain's fit. Code: `crates/locus-analysis/src/bloodstain.rs` (pure, tested), commands in `src-tauri/src/bloodstain_cmds.rs`, report in `crates/locus-report/src/bloodstain.rs`, UI in `app/src/tools/bloodstain/`. Validated against ground truth from `locus-validate gen-bloodstain` (`crates/locus-validate/tests/bloodstain.rs`).

**Straight-line paths ignore gravity and air drag.** Real droplets fall along curved paths, so a straight-line origin tends to be too high. The app and the report say so, and by default only stains clearly moving upward at impact are used (below).

## Inputs

For each stain, a photograph from the project's evidence (an image file, hashed on import like every evidence file) and:

- **Alignment.** Two or more point pairs: a pixel in the photo (a scale mark, a corner, a fiducial) and the same point clicked on the scan. Each click is resolved again in Rust from the stored scan data, and the scan point (scan, record, cleanup revision) is recorded. A plane is fitted to the scan within 10 cm of the pairs, with its normal on the side the examiner is viewing from, captured when the pairs are picked. The photo is placed on that plane by a similarity (scale, rotation, shift; no mirror image, no perspective) fitted to the pairs by least squares (the closed-form 2-D Procrustes solution). With three or more pairs, the residuals check it; two pairs fix it exactly and are reported as such. The photo can be shown on the scan with an adjustable transparency to check the alignment by eye. It is drawn lifted off the surface toward the viewer by three times the RMS roughness of the scan there (at least 1 mm), so the scan points don't show through it.
- **Perspective (optional).** A photo not taken square on is first corrected by the homography taking four corners of a rectangle on its scale, clicked in order around it, onto that rectangle's stated size (Hartley and Zisserman 2004, the four-point solution on normalised coordinates). The corrected photo keeps the scale's own mean resolution, and the similarity above is then fitted to the corrected pixels. The correction is never allowed to mirror the photo. Its size is reported as the *stretch*: the ratio of the correction's largest to smallest local scale at the stain, minus 1 (0 square on; about 1/cos θ − 1 for a photo θ off square on). A stretch over 10 % (about 25°) is flagged in "Needs attention": the result then rests on four clicks, and a square-on photo is better evidence. The scale's stated size is also checked against the scan: the photo's resolution from the pairs over that from the scale should be 1. It is flagged when it is off by more than 2 % and by more than 2.5 × the pairs' own scale uncertainty. For a similarity, that uncertainty is relative and equals the rotation's in radians, so scan points that snap within a 3 mm spacing on a 35 mm scale can alone be several per cent off.
- **Edge.** Either automatic, or clicked. Automatic: the region darker than a threshold connected to a clicked seed (4-connected flood fill), and its boundary to a fraction of a pixel, where the brightness crosses the threshold, interpolated linearly between neighbouring pixel centres. The threshold starts halfway between the seed's brightness and the median of the crop's border, and is adjustable. The webview decodes the photo and sends a greyscale crop (at most 800 px a side, downsampled from up to 2400 px) to the Rust edge finder. The edge points, seed and threshold are stored with the run, so the fit can be repeated from the record.
- **Tail.** A click at the tip of the tail, which sets which way along the long axis the droplet travelled.
- **Exclusion.** Any stain can be left out with a reason, which is stored and printed.

## Ellipse

An ellipse (centre, semi-axes, orientation) is fitted to the edge points on the surface's plane, in metres, by Levenberg–Marquardt. Each point's residual is its scaled algebraic distance, (√(u²/a² + v²/b²) − 1) × (a + b)/2, which approximates the geometric distance near the ellipse (Ahn et al. 2001). Points well off the ellipse (the tail, a satellite touching the stain) are left out repeatedly: those beyond 3 × 1.4826 × the median absolute residual (a robust 3σ) and more than a pixel. The axes' and orientation's 1σ come from the fit's covariance scaled by its residual variance. Added in quadrature is the edge method's own systematic error, measured on the generator's photos: 0.2 % of each axis and 0.1° on the orientation. Without it, the origin's χ² was 7 × its degrees of freedom and the origin 2.5 mm low. With it, χ² is about half its degrees of freedom and the error under 1 mm. For a photo corrected for perspective, the corner clicks' error (1σ = 1 px unless stated) is added too, found by Monte Carlo: the four corners are redrawn 64 times, the photo re-corrected and re-aligned, and the ellipse refitted to the same edge points (at most 300 of them, evenly spaced). The spread of width, length and orientation over the redraws is added in quadrature.

## Impact angle and direction

- **Impact angle** α = asin(w / l), for width w and length l (Balthazard et al. 1939; Bevel and Gardner 2008). Its 1σ follows from the ratio's: σ_α = σ_(w/l) / cos α, with σ_(w/l) = (w/l) √((σ_w/w)² + (σ_l/l)²). This grows without bound as the stain gets round, so it is capped at 90°.
- **Direction of travel** γ, along the long axis toward the tail. On a wall: clockwise from straight up, seen facing the wall. On a floor: clockwise from the reference axis (project north, +y, or a named axis at a stated angle from it). Its 1σ combines the ellipse's orientation error and the photo's rotation error in quadrature. The rotation error is σ_point / √Σ|p_i − p̄|² for scan-point 1σ σ_point (the project's point uncertainty, or the pairs' own scatter if larger) and pair positions p_i on the plane. A Monte Carlo test checks it within 10 %.
- **The path back:** from the stain, r = sin α n − cos α t, for the surface normal n (toward the blood's side) and the travel direction t in the surface.

## Which stains are used

A stain is used only if its droplet was **clearly moving upward** at impact: a wall stain whose direction is within 90° − 2σ_γ of straight up. The rule is from the validation. With the generator's hand-measurement noise, 14 % of the stains whose measured direction pointed upward were really moving downward. Their paths go back below the origin and pulled it about 20 cm low, a bias no resampling can see. Requiring the direction to be upward by 2σ removes them. Floor stains, downward-moving stains and stains whose direction is too uncertain are left out unless the examiner includes them all with a stated reason, which is stored, printed and warned about. The spec asks for stains "the analyst marks as upward-moving". The analyst marks each stain's tail, and the rule applies the direction that tail gives, with its uncertainty.

## The origin

The origin x minimises

  Σ_i [ (α_i(x) − α_i)² / σ_α,i² + (γ_i(x) − γ_i)² / σ_γ,i² ]

where α_i(x) and γ_i(x) are the impact angle and direction a droplet travelling straight from x would have had at stain i. That is, the point whose paths best match every stain's measured angles, each in units of its own 1σ. It is solved by Levenberg–Marquardt, starting from the point nearest all the paths by plain least squares (Σ (I − r rᵀ)(x − c) = 0). If the paths are nearly parallel (the smallest eigenvalue of the mean of I − r rᵀ below 10⁻⁶), the run is refused.

### Why the fit is in angles, and the conventional point beside it

The fit in the stains' angles is the maximum-likelihood point when each stain's impact angle and direction have independent normal errors of the stated 1σ. It weights each stain by how well it was measured, as Camana (2013) does for the directions in plan. The conventional area-of-origin calculation, the least-squares point nearest all the paths by perpendicular distance with every stain alike (the directional analysis described by Carter 2001 and Bevel and Gardner 2008), is computed on the same stains and shown beside it in the app and the report. It gets its own 95 % region from the same bootstrap resamples, and its distance from the result. It is not the result because an error θ in a stain's direction does not scatter its path symmetrically about the true origin. A path turned by θ about the surface normal misses the origin, at distance d, on one side only, by d sin α cos α (1 − cos θ). Errors of that kind pull the conventional point consistently in one direction, and resampling cannot show it. Illes and Boué (2013) note the same weakness of plain averaging over stains and use robust estimators (a trimmed mean, RANSAC) instead. The fit in angles has no such term.

On the same stains used (the validation below): from the generator's photos, the fit in angles is 0.6 mm from the truth on average and its region covers the truth 8 of 8 times; the conventional point is 4.8 mm off and its region covers the truth 0 of 8 times. With hand-measurement noise (40 rooms), 233 mm against 302 mm, with coverage 95 % against 92 %. Earlier development runs with the weaker "upward" rule put the conventional point 12 to 32 cm off.

### Near-round stains

Each stain's share of the fit is its share of the information at the origin: the summed squares of the derivatives of its two misfits (in σ units) with respect to the origin's position, over the total for the stains used. It is shown for every stain. When stains with impact angles of 70° or more carry over half of it, the run is flagged in "Needs attention" and those stains are named. Their width over length, and so their angles, are poorly determined, and the origin rests mostly on them.

Reported with it:

- each stain's **residual**: the origin's distance from its path (m), and in units of the path's uncertainty there (√χ², 2 degrees of freedom);
- **χ²** on 2n − 3 degrees of freedom. When χ²/dof exceeds 2, the report warns that some stains may not come from this origin, or were measured worse than stated;
- stains whose path points away from the origin (the origin behind them), with a warning.

## Floor stains: convergence in plan (optional)

Floor stains are never "upward", so they are never used for the origin. On request, a separate 2-D result is computed from the floor stains not excluded (normal within about 25° of straight up): the point in plan whose bearing from each stain best matches that stain's measured direction back along its path, in units of the direction's 1σ (Levenberg–Marquardt from the lines' least-squares point). It uses no impact angles and gives no height. It is reported in its own section, with the conventional point (least squares on the lines' perpendicular distances) for comparison, χ² on n − 2 degrees of freedom, each stain's distance from its line, and a 95 % ellipse from 2,000 bootstrap resamples with Hotelling's radius for p = 2. It is never mixed into the 3-D origin. At least 4 floor stains are needed; with fewer, the report says so. In a unit test with 20 floor stains and 3° direction noise, the ellipse covered the truth in 92.5 % of 200 runs. The 3-D origin must still be computable; a run with floor stains only is not yet supported.

## Uncertainty: the 95 % region

The origin is recomputed on 2,000 bootstrap resamples (the stains used, drawn with replacement; seeded, so a run repeats exactly; Efron and Tibshirani 1993). The covariance C of the resampled origins gives an ellipsoid (x − x̂)ᵀ C⁻¹ (x − x̂) ≤ q centred on the estimate. The radius q comes from Hotelling's T² for p = 3 and n stains used, because C is estimated from the same stains: q = p n / (n − p) · F_(p, n−p)(0.95) (Johnson and Wichern 2007, §5.4). It tends to χ²₃(95 %) = 7.81 for many stains and grows for few (18.6 for n = 10). A χ² radius with 30 stains covered the truth only 88 % of the time; with q, 94.5 % (the unit test, 200 runs). At least 4 stains are required. The F quantile is computed from the regularised incomplete beta function (continued fraction) and checked against the NIST/SEMATECH tables.

The ellipsoid shows the scatter among the stains used. It does not include bias from the straight-line model, the photo alignment, or stains chosen from one side of the pattern.

## Validation

The Phase 6 acceptance criterion is mean error under 10 cm on clean synthetic data. The generator throws 300 droplets from (2.2, 1.6, 1.1) m in a 4 × 3.5 m room. It keeps stains 1.8–5.5 mm wide with impact angles 10–80° away from corners, measures their ellipses with noise, and renders each stain's photo (20 px/mm, a tail, three fiducials).

| Case | Fit in angles (the result) | Conventional point, same stains |
|---|---|---|
| Exact stains (unit test) | the origin exactly (1e-9 m) | the origin exactly |
| From the generator's photos: alignment by the fiducials, automatic edges, the marked tail (8 rooms, about 80 stains used each) | mean error **0.6 mm**, worst 1.0 mm; the truth inside the 95 % region in 8 of 8 | 4.8 mm, worst 5.9 mm; inside its region 0 of 8 |
| In the app: six stain photos aligned by clicking their fiducials on the cloud, automatic edges, tails clicked | **11.5 mm** from the truth, 95 % region 8 × 5 × 3 cm, χ² 0.4 on 9 | 8 mm from the result |
| In the app: the same six photos warped as if taken 35° off square on, corrected by the scale square's corners | 27 mm from the truth, impact angles within 1° of the truth, 95 % region 44 × 22 × 16 cm, χ² 9.4 on 9 | 12 mm from the result |
| Hand-measurement noise (0.1 mm + 1.5 % per edge, 40 rooms; mostly near-round stains, direction errors of 15–40°) | mean error 233 mm, worst 500 mm, about 14 stains used; the truth inside the 95 % region **95 %** of the time | 302 mm, worst 745 mm; inside its region 92 % |
| Under gravity (ballistic flight with drag, 3–8 m/s), every stain included | the straight-line origin 1.7–2.4 m too high | |
| Perspective (unit test): a stain and a 50 mm scale photographed 35° off square on from 0.4 m | width and length within 1 %, orientation within 0.5°; the photo cannot be placed without the correction | |

The last case is why the restriction exists. At these speeds almost no droplet is still rising when it lands (1 of 265 in the generator's room), so real patterns may leave few usable stains.

## Assumptions

These are stored with every run and printed in its report.

- Each droplet travelled in a straight line from the origin to its stain. Gravity and air drag are ignored.
- Each stain is the ellipse of a spherical droplet striking a flat, smooth surface. Width over length is the sine of the impact angle, and the long axis lies along the direction of travel, toward the tail.
- The droplets came from one origin, at about the same time.
- Each photo is flat on the stain's surface, and either taken square on to it or corrected for perspective from a rectangular scale lying in that plane.
- Only wall stains clearly moving upward at impact are used, unless the examiner has included the others with a stated reason.

## Limitations

These are also stored with every run and printed in its report.

- **Straight lines.** The straight-line origin is usually too high; its height is best read as an upper bound.
- **Round stains.** Width over length is sensitive to measurement above about 70° impact, and the direction of a nearly round stain is poorly defined.
- **Surfaces and stains.** Rough, absorbent or textured surfaces, satellite spatter, and stains that ran, dried unevenly or overlap distort the ellipse.
- **The region.** The ellipsoid is from resampling the stains used. It does not include bias from the straight-line model, the alignment, or the choice of stains.
- **What it says.** The origin is where the paths pass closest together. It does not say what caused the pattern, or how many events there were.
- **Photos.** A photo taken at an angle to the surface must be corrected from a scale in the plane of the surface. The correction assumes a pinhole camera: lens distortion is not modelled, so take stain photos square on where possible.

## References

- V. Balthazard, R. Piédelièvre, H. Desoille and L. Derobert, "Étude des gouttes de sang projeté", *Annales de médecine légale*, 19, 1939: the sine relation between a stain's shape and its impact angle.
- T. Bevel and R. M. Gardner, *Bloodstain Pattern Analysis with an Introduction to Crime Scene Reconstruction*, 3rd ed., CRC Press, 2008: impact angle, directionality, area of origin and its limits.
- F. Camana, "Determining the area of convergence in bloodstain pattern analysis: a probabilistic approach", *Forensic Science International*, 231, 2013, 131–136: weighting stains' directions by their uncertainty, in plan.
- M. Illes and M. Boué, "Robust estimation for area of origin in bloodstain pattern analysis via directional analysis", *Forensic Science International*, 226, 2013, 223–229: the weakness of plain averaging over stains, and robust alternatives.
- R. Hartley and A. Zisserman, *Multiple View Geometry in Computer Vision*, 2nd ed., Cambridge University Press, 2004: the four-point homography and its normalisation.
- A. L. Carter, "The directional analysis of bloodstain patterns: theory and experimental validation", *Canadian Society of Forensic Science Journal*, 34(4), 2001: straight-line (tangent) reconstruction and its bias under gravity.
- D. Attinger et al., "Fluid dynamics topics in bloodstain pattern analysis: comparative review and research opportunities", *Forensic Science International*, 231, 2013: droplet flight, drag and the limits of straight-line methods.
- S. J. Ahn, W. Rauh and H.-J. Warnecke, "Least-squares orthogonal distances fitting of circle, sphere, ellipse, hyperbola, and parabola", *Pattern Recognition*, 34, 2001.
- B. Efron and R. J. Tibshirani, *An Introduction to the Bootstrap*, Chapman & Hall, 1993.
- R. A. Johnson and D. W. Wichern, *Applied Multivariate Statistical Analysis*, 6th ed., Pearson, 2007: Hotelling's T² confidence regions.
- NIST/SEMATECH e-Handbook of Statistical Methods, §1.3.6.7.3 (upper critical values of the F distribution).
