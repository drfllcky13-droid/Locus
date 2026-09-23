# Bullet trajectory

The path of a projectile through one or more surfaces, from the defects it left or from a probe rod, with its uncertainty, its angles to each surface, and where a muzzle could have been. Code: `crates/locus-analysis/src/trajectory.rs` (pure, tested), commands in `src-tauri/src/analysis_cmds.rs`, report in `crates/locus-report/src/trajectory.rs`, UI in `app/src/tools/trajectory/`. Validated against ground truth from `locus-validate gen-trajectory` (`crates/locus-validate/tests/trajectory.rs`).

## Inputs

- **Defects:** a click inside or beside each defect on the point cloud: entry and exit on each surface, **in the order the projectile travelled**. The examiner names each surface. Each click is resolved again in Rust from the stored scan data, as for measurements, and recorded with its source (scan, record number in the source file, cleanup revision). The hole's centre is then fitted from its rim (below). The examiner can instead use the clicked point as the centre, with a stated 1σ and a **reason**, which is stored with the run and printed in the report. For each defect, a plane is fitted by total least squares to the scan's points within a radius (default 5 cm), for the angles to that surface. A photograph of the defect can be attached from the project's image evidence; the report prints it and its hash, checked against the evidence record.
- **Probe rod:** two points well apart along a rod placed through the holes, with the rod's **play**: how far it can tilt in its hole (degrees, entered by the examiner). A rod in a hole of diameter D through material of depth t can tilt by up to atan((D − d_rod)/t). In thin material that is many degrees, which is why defects on several surfaces are preferred.

## Hole centres

The centre of a hole has no scan points, so a click lands on the rim or beside it. `crates/locus-analysis/src/defect.rs` finds the centre from the points within a search radius of the click (default 3 cm):

1. A plane is fitted to them, and only the clicked face's layer is kept (points within a few spacings of that plane), so the far face of a thin panel doesn't blur the rim.
2. The point spacing s is the median distance to the 4th-nearest neighbour.
3. The hole is the largest empty circle in the plane near the click (grid search, step max(s/2, reach/60)). A click with no hole near it, or a hole open to one side over more than 45°, is refused, with a message suggesting a manual centre.
4. Rim points: round the hole in sectors of about one spacing (12–72 sectors), the nearest point in each.
5. An ellipse is fitted to the rim points by Levenberg–Marquardt (centre, semi-axes, orientation). Rim points sit up to half a spacing outside the true edge, so both semi-axes are reduced by s/2. The quantisation error s/√12 is added to the σ of the centre and of both axes, in quadrature with the fit's own covariance.

The fitted centre and its σ are used in the line fit in place of a stated σ.

**The ellipse cross-check.** A round projectile striking at impact angle α leaves an ellipse with b/a ≈ sin α (Haag and Haag 2011). The ellipse gives an impact angle asin(b/a) independent of the path, and the report sets it beside the path's impact angle at that face. They are tested on the ratio: |b/a − sin α_path| ≤ 1.96 √(σ²_ratio + (cos α_path · σ_α)²). The test uses the ratio because the ratio's errors are close to normal, while through asin they are strongly skewed as b/a approaches 1. A disagreement is flagged as a reason to check for deflection, a damaged or non-elliptical hole, or the wrong hole. It does not change the path.

With few rim points the ellipse angle is weak. The generator's 9 mm holes at 2 mm point spacing give about 15 rim points and impact σ of 15° to 90°. The centres are still good: within 0.9 mm of truth, σ 0.6 mm.

## The path

A straight line is fitted by weighted total least squares. With weights w_i = 1/σ_i², it passes through the weighted centroid c = Σ w_i p_i / Σ w_i along the principal eigenvector of the weighted scatter Σ w_i (p_i − c)(p_i − c)ᵀ, which minimises Σ w_i d_i² for perpendicular distances d_i. The direction is oriented from the first point toward the last.

- **Bearing:** clockwise from the chosen reference axis (default project north, +y; the examiner can name another axis and give its angle clockwise from +y). **Elevation:** up or down from level, asin(d_z).
- **Residuals:** each point's perpendicular distance from the line.

## Uncertainty

The direction's covariance is propagated to first order from every input coordinate (a numerical Jacobian of the whole fit), each point with its stated isotropic σ (JCGM 100:2008, the GUM, §5.1). The covariance has rank 2: it lies in the plane perpendicular to the direction.

- **χ² check.** χ² = Σ (d_i/σ_i)² with 2n − 4 degrees of freedom (a 3-D line has four parameters, and each point contributes two perpendicular residuals). If χ²/dof exceeds 1, the points scatter more than their stated uncertainty. The covariance is then multiplied by χ²/dof (the Birge ratio) and the report says so. With two points there is no redundancy and no check.
- **Rod play** is added in quadrature, the same in every direction perpendicular to the rod.
- **The 95 % cone:** half-angles √(χ²₂(95 %)) · √λ for the covariance's two principal values λ, where χ²₂(95 %) = 5.991. It is elliptical in general. The larger half-angle is shown.
- **Bearing and elevation σ** follow from the covariance through their gradients.

Tests: a Monte Carlo of 4,000 noisy repeats agrees with the stated σ of bearing and elevation within 10 %. Against the generator (three panels, 2 mm in-plane picking noise plus 1 mm scan noise), the true direction lies inside the stated 95 % cone in 94.5 % of 400 runs.

## Angles to each surface

For each named surface with a fitted plane, the normal n is taken on the shooter's side. With h, the surface's horizontal axis (the shooter's right, up × n), and v = n × h:

- **impact angle** = asin(−d · n): 90° is square on;
- **horizontal angle** = atan2(d · h, −d · n): + to the shooter's right;
- **vertical angle** = atan2(d · v, −d · n): + upward.

For a horizontal surface, h is +x and v is n × x. The uncertainties come from the direction's covariance. The plane's own uncertainty is not included; its RMS residual is reported.

**Convention.** The report uses one of two conventions, chosen per run and stated in its method section:

- **Level and perpendicular** (default): vertical angle up or down from level (the path's elevation); horizontal angle left or right of perpendicular to the surface, in plan, viewed facing the surface from the side the bullet came from. A floor or ceiling has no horizontal angle.
- **Surface normal:** the horizontal and vertical angles above, in the surface's own frame.

The impact angle is given in both.

## Possible muzzle positions

The path is traced back from the first point through a **height band** above a floor elevation the examiner sets or picks (default 0.9–1.8 m), up to a maximum range (default 30 m). The report gives:

- where the centre line is inside the band (distances back from the first point, and the points);
- two zones, in the band, as plan footprints (the convex hull of where 72 of each cone's edges cross the band):
  - **Measurement uncertainty (95 %, computed)**: the fit's own 95 % cone, elliptical. Blue in the report and the 3D view.
  - **Examiner-defined zone (±X°, analyst judgment)**: a circular cone the examiner sets (default ±5°) for what the measurement doesn't cover, such as deflection or a doubtful defect. Orange.

If the computed cone is wider than the examiner's zone, the report says so.

## Report

Built from the stored run only. It contains:

- the case number (a project setting), project, record, hash, examiner and audit head;
- warnings;
- the result, scene-relative;
- the angles to each surface in the chosen convention;
- the points used, each with its centre method, coordinates, σ, residual and source scan point;
- the hole fits, with the cross-check and any manual centres and their reasons;
- plan and elevation figures showing both zones;
- defect photographs;
- method, assumptions, limitations;
- sign-off lines for the examiner and technical review.

## Validation

The acceptance criterion is recovering a synthetic multi-surface trajectory within 0.5°. The generator's default scene has a vehicle door (1 mm, 35° incidence), an interior wall (12.5 mm, 22°) and a wardrobe side (18 mm, 12°), 4.3 m apart. With 2 mm picking noise, the worst direction error over 50 seeded runs is 0.066°.

In the app, the defects of the same scene (2 mm point spacing) were clicked 8 mm to the side of each hole's centre and the centres fitted from the rims. The result was 0.008° from the true direction (χ² 0.97 on 8 degrees of freedom), and all six ellipse cross-checks agreed. Before hole fitting, the clicked points themselves gave 0.105°, and the χ² test widened the uncertainty 2.4×. The same fit in `crates/locus-validate/tests/trajectory.rs` puts every centre within 1.5 mm of truth.

A probe rod through the wardrobe side (a random tilt within its play of up to 9.3°) is recovered within its play. The cone always covers the play.

## Assumptions

These are stored with every run and printed in its report.

- The projectile travelled in a straight line between the first and last points used. Over the short distances between defects, gravity drop and deflection are negligible.
- Each defect's centre (fitted or picked) is where the projectile passed through that face of the surface, and the points are in travel order.
- A fitted hole centre is the centre of an ellipse fitted to the hole's rim as sampled by the scan.
- Each point's uncertainty is independent and isotropic, as stated.
- The small uncertainty of each fitted plane is not included in the surface angles.

## Limitations

These are also stored with every run and printed in its report.

- **Deflection.** A surface, or a harder layer behind it, can change the path. Points on either side of a deflection must not be fitted as one line. A large χ² is a warning sign, not proof either way.
- **One thin surface.** Entry and exit on one thin surface define the direction poorly; the cone shows how poorly.
- **Where the shooter was.** Tracing back gives where a muzzle could have been if the path continued straight. It does not say the shooter stood there, and the height band is the examiner's assumption.
- **Which shot.** The fit says nothing about the order of shots, or which shot made which defect.
- **Hole shape.** Exit holes, tears, spalling, and deformed or tumbling bullets make holes that are not ellipses. The ellipse angle is then only a cross-check, and a fitted centre may be wrong: use a manual centre with a reason. A manual centre's σ should be at least the hole's radius when the click is on the rim.
- **Point spacing.** A hole only a few point spacings across gives few rim points: a centre σ near the spacing, and an ellipse angle too weak to test the path much.

## References

- JCGM 100:2008, *Evaluation of measurement data — Guide to the expression of uncertainty in measurement* (GUM). JCGM 101:2008, *Supplement 1: Propagation of distributions using a Monte Carlo method*.
- L. C. Haag and M. G. Haag, *Shooting Incident Reconstruction*, 2nd ed., Academic Press, 2011: trajectory determination from defects and probes, and deflection.
- G. H. Golub and C. F. Van Loan, *Matrix Computations*, 4th ed., Johns Hopkins, 2013: total least squares.
