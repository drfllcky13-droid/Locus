# Bullet trajectory

The path of a projectile through one or more surfaces, from the defects it left or from a probe rod, with its uncertainty, its angles to each surface, and where a muzzle could have been. Code: `crates/locus-analysis/src/trajectory.rs` (pure, tested), commands in `src-tauri/src/analysis_cmds.rs`, report in `crates/locus-report/src/trajectory.rs`, UI in `app/src/tools/trajectory/`. Validated against ground truth from `locus-validate gen-trajectory` (`crates/locus-validate/tests/trajectory.rs`).

## Inputs

- **Defects:** the centre of each defect, picked on the point cloud: entry and exit on each surface, **in the order the projectile travelled**. The examiner names each surface and gives each pick's 1σ uncertainty (default 2 mm). Each pick is resolved again in Rust from the stored scan data, as for measurements. For each defect, a plane is fitted by total least squares to the scan's points within a radius (default 5 cm), for the angles to that surface.
- **Probe rod:** two points well apart along a rod placed through the holes, with the rod's **play**: how far it can tilt in its hole (degrees, entered by the examiner). A rod in a hole of diameter D through material of depth t can tilt by up to atan((D − d_rod)/t). In thin material that is many degrees, which is why defects on several surfaces are preferred.

## The path

A straight line is fitted by weighted total least squares. With weights w_i = 1/σ_i², it passes through the weighted centroid c = Σ w_i p_i / Σ w_i along the principal eigenvector of the weighted scatter Σ w_i (p_i − c)(p_i − c)ᵀ, which minimises Σ w_i d_i² for perpendicular distances d_i. The direction is oriented from the first point toward the last.

- **Bearing:** clockwise from project +y. **Elevation:** up from horizontal, asin(d_z).
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

## Possible muzzle positions

The path is traced back from the first point through a **height band** above a floor elevation the examiner sets or picks (default 0.9–1.8 m), up to a maximum range (default 30 m). The report gives:

- where the centre line is inside the band (distances back from the first point, and the points);
- the plan footprint of everything within a **cone** of the stated half-angle (default ±5°, configurable) that is inside the band: the convex hull of where 72 of the cone's edges cross the band.

If the fit's own 95 % cone is wider than the stated cone, the report says so.

## Validation

The acceptance criterion is recovering a synthetic multi-surface trajectory within 0.5°. The generator's default scene has a vehicle door (1 mm, 35° incidence), an interior wall (12.5 mm, 22°) and a wardrobe side (18 mm, 12°), 4.3 m apart. With 2 mm picking noise, the worst direction error over 50 seeded runs is 0.066°.

In the app, the defects of the same scene were picked on its point cloud, clicking 8 mm to the side of each hole's centre. The result was 0.105° from the true direction. The χ² test showed the clicks were farther from the centres than the stated 2 mm, and widened the uncertainty 2.4×.

A probe rod through the wardrobe side (a random tilt within its play of up to 9.3°) is recovered within its play. The cone always covers the play.

## Assumptions

These are stored with every run and printed in its report.

- The projectile travelled in a straight line between the first and last points used. Over the short distances between defects, gravity drop and deflection are negligible.
- Each defect centre was picked where the projectile passed through that face of the surface, and the points are in travel order.
- Each point's uncertainty is independent and isotropic, as stated.
- The small uncertainty of each fitted plane is not included in the surface angles.

## Limitations

These are also stored with every run and printed in its report.

- **Deflection.** A surface, or a harder layer behind it, can change the path. Points on either side of a deflection must not be fitted as one line. A large χ² is a warning sign, not proof either way.
- **One thin surface.** Entry and exit on one thin surface define the direction poorly; the cone shows how poorly.
- **Where the shooter was.** Tracing back gives where a muzzle could have been if the path continued straight. It does not say the shooter stood there, and the height band is the examiner's assumption.
- **Which shot.** The fit says nothing about the order of shots, or which shot made which defect.
- **Picking a hole's centre.** The centre of a hole has no scan points, so a pick lands on the rim or beside it. The tool uses the picked point as given. State its uncertainty accordingly: at least the hole's radius when picking on the rim.

## References

- JCGM 100:2008, *Evaluation of measurement data — Guide to the expression of uncertainty in measurement* (GUM). JCGM 101:2008, *Supplement 1: Propagation of distributions using a Monte Carlo method*.
- L. C. Haag and M. G. Haag, *Shooting Incident Reconstruction*, 2nd ed., Academic Press, 2011: trajectory determination from defects and probes, and deflection.
- G. H. Golub and C. F. Van Loan, *Matrix Computations*, 4th ed., Johns Hopkins, 2013: total least squares.
