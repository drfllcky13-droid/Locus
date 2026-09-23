# Camera matching, subject height and witness perspective

This method recovers the camera that took a photo or CCTV frame from points matched between the image and the scan. It then measures a subject's height by reverse projection, and tests a witness's lines of sight against the scan. Every result carries its uncertainty.

- Code: `crates/locus-analysis/src/camera.rs` (pure, tested).
- Commands: `src-tauri/src/camera_cmds.rs`.
- Reports: `crates/locus-report/src/camera.rs`.
- UI: `app/src/tools/camera/`, plus the viewer's camera-match mode in `app/src/viewer3d/engine.ts`.
- Validated against ground truth from `locus-validate gen-camera` (`crates/locus-validate/tests/camera.rs`).

## The camera model

The model is OpenCV's, the same as the generator's (Brown 1966).

- **Axes.** Camera x is right, y is down and z is forward. A point X maps to `x_c = R (X − C)` and normalised coordinates `(a, b) = (x_c/z_c, y_c/z_c)`.
- **Distortion.** With `r² = a² + b²`:
  - `a' = a(1 + k1 r² + k2 r⁴ + k3 r⁶) + 2 p1 a b + p2 (r² + 2a²)`
  - `b' = b(1 + k1 r² + k2 r⁴ + k3 r⁶) + p1 (r² + 2b²) + 2 p2 a b`
- **Pixels.** `u = f a' + cx` and `v = f b' + cy`. Pixels are square with no skew, and pixel centres sit at integer + 0.5.
- **Fold radius.** A strong barrel distortion polynomial stops increasing at some radius and folds back. Beyond that radius a point outside the field of view would land inside the image, so the model is only used inside it. The limit is where `1 + 3k1 r² + 5k2 r⁴ + 7k3 r⁶` reaches 0. The generator had exactly this bug, found by the validation: a marker 67° off-axis appeared in the middle of the CCTV frame.

## Solving the camera

**Inputs.** Pairs of a pixel and the same point on the scan. Each scan point is resolved again from stored data, and its scan, record and cleanup revision are recorded.

- At least 6 pairs are needed, and more than half the lens model's unknowns.
- The pairs must not lie on one plane (the smallest over the largest eigenvalue of their scatter must be at least 10⁻⁴).
- The photo is an image in the evidence, named by its record and hash.

**Lens models.** The examiner chooses one:

| Model | Unknowns besides the pose (6) |
|---|---|
| Focal length only | f |
| + k1 | f, k1 |
| + k1, k2 | f, k1, k2 |
| Full | f, cx, cy, k1, k2, k3, p1, p2 |

The simpler models hold the principal point at the image centre. The validation shows the cost: a principal point 4 px off centre, held at the centre, biased heights by about a centimetre. Use the full model when there are enough pairs (at least as many as its 14 unknowns); the solve warns when there are fewer.

**Start.** The direct linear transform on the pairs nearest the image centre, where distortion is least: at least 8 of them, or half, provided they are off one plane (Hartley and Zisserman 2004, §4.4 and §7.1).
- The pixels and points are normalised, and the smallest right singular vector gives P.
- P is decomposed into K, R and C. The RQ step uses the Cholesky factor of M Mᵀ through the exchange matrix.
- The sign is fixed so the points are in front of the camera, and a mirror-image result is refused.

**Refinement.** Levenberg–Marquardt on the reprojection errors, each in units of its σ. That σ combines the pick σ (pixels) with the scan point's σ projected at its depth: `σ² = σ_px² + (f σ_point / z)²`.
- The refinement runs in stages: the pose and focal length, then k1, then k2, then the rest, as far as the chosen model goes. Each stage starts from the previous one, with the weights set twice per stage.
- Started directly with every parameter free from the DLT (which has no distortion), the CCTV case settled in a wrong minimum at 6 px RMS.

**Uncertainty.** The covariance is (JᵀJ)⁻¹ at the solution, multiplied by the Birge ratio squared, χ²/dof, when that exceeds 1 (Birge 1932).
- Reported: position, heading/pitch/roll (propagated to first order) and focal length, principal point and distortion, each with 1σ; each pair's residual in pixels and in σ; RMS and χ².
- Warnings when:
  - χ²/dof exceeds 4;
  - a pair is more than 3σ out;
  - the focal length's σ exceeds 5 %;
  - the pairs number fewer than the unknowns.

## Subject height by reverse projection

The examiner marks two image points (Criminisi, Reid and Zisserman 2000): the floor midway between the feet, or under the body's centre in a stride, and the top of the head.

- The feet ray meets the floor plane z = floor, which the examiner states or picks on the scan.
- The height is where the head ray passes closest to the vertical through that floor point.
- The head ray's distance from the vertical there is reported. A large value (over 5 cm is warned) means the subject was not upright over the feet point, or the feet point is off.

**Uncertainty.** A Monte Carlo of 2,000 draws, seeded so a run repeats exactly.
- Each draw takes the camera's free parameters from their covariance (Cholesky; negative eigenvalues clamped) and moves both image points by the pick σ.
- The 1σ and the 95 % interval (2.5th and 97.5th percentiles) are reported.

**Person model.** The app draws a person model of the measured height over the photo, projected through the solved camera with its distortion, and in the 3D view.
- The examiner can set the model's height, pose and facing to match the frame by eye.
- They can then use the model's projected top of head as the head point. The report says the head point came from a matched model of that stature, not a click.

**The model's proportions.** Stature fractions from Drillis and Contini (1966), as reproduced in Winter (2009, Fig. 4.1):

| Point or segment | Fraction of stature |
|---|---|
| Ankle | 0.039 |
| Knee | 0.285 |
| Hip | 0.530 |
| Shoulder | 0.818 |
| Chin | 0.870 |
| Upper arm | 0.186 |
| Forearm | 0.146 |
| Hand | 0.108 |
| Shoulder width | 0.259 |
| Hip width | 0.191 |
| Foot length | 0.152 |
| Foot width | 0.055 |

Stature is floor to top of head, standing, without footwear. The model is exactly its stature, soles to the top of the head, and the generator uses the same fractions.

**What is measured.** The height of the top of what was marked, in the posture shown. Hair, headwear, footwear, a stride, a slouch or a head tilt all change it by centimetres, and none of these is corrected for.

## Photo over the 3D scene

The viewer can look through the solved camera: its pose, and a projection built from f, cx and cy with the photo fitted into the view. The photo is drawn over the scene with adjustable opacity.

- The photo is drawn through the lens model: a shader takes each screen pixel's undistorted image position, applies the distortion, and samples the photo there. Straight edges in the scan then line up with the photo's curved ones.
- In the photo editor, the scan's points (a sample of those loaded) are drawn over the photo through the solved camera, with each pair's residual drawn ten times longer.

## Witness perspective

The examiner states the eye position, as a floor point picked on the scan and an eye height above it, the point the witness was looking toward, and a horizontal field of view. The 3D view can be set to that eye.

**Line-of-sight test.** From the eye to each picked target:
- the line is blocked where at least 3 scan points lie within 30 mm of it, ignoring the first and last 100 mm (the surfaces its ends are on);
- the first obstruction and its distance from the eye are reported;
- the scan points near the line are gathered by balls along it, each point counted once.

Only what was scanned can block a line. People, vehicles, lighting and visibility at the time are not modelled, and gaps in the scan (glass, dark surfaces, occlusion) can make a blocked line look clear.

## Validation

Ground truth comes from `locus-validate gen-camera`, rendered in an 8 × 6 × 3 m room with control markers and three standing people (1.63, 1.84 and 1.72 m). There are two cameras:
- **CCTV:** high in a corner, 1280 × 720, f = 620 px, k1 = −0.28, k2 = 0.09, k3 = −0.012, principal point 6 px off centre;
- **Handheld photo:** 1600 × 1200, f = 1150 px, mild distortion.

Markers are picked with 0.5 px noise, and heads and feet with 1 px.

| Case | Result |
|---|---|
| Exact pairs, full model (unit test) | camera exactly (position to 10⁻⁶ m, distortion to 10⁻⁵) |
| Noisy pairs, 40 seeds (unit test) | position error in σ units: mean z² within 0.6–1.6 (the stated σ is honest) |
| 40 rooms × 2 cameras × 3 people, full model, about 25 pairs a camera | height mean error **6.0 mm**; with a good solve (height 1σ ≤ 10 mm, 211 of 240), 95 % of errors under **14 mm**, worst 28 mm; truth inside the 95 % interval **92 %** of the time |
| In the app: 14 pairs clicked on the CCTV frame and the cloud, full model, three people's feet and heads clicked | camera 57 mm from the truth (1σ 102, 58, 53 mm); heights +9.8, +11.6 and +7.3 mm (1σ 16, 17, 13 mm) |

The spec's bound is "height under 2 cm with a good camera solve". Read as 95 % of errors under 2 cm when the solve states the height to 1 cm, it is met: 14 mm.

## Assumptions

These are stored with every run and printed in its report.

- The image is a single central projection (a pinhole camera) with the distortion of the chosen lens model, square pixels and no skew.
- The scene has not changed between the image and the scan at the paired points.
- A subject's feet point is on the floor plane, and the top of the head is vertically above it.

## Limitations

These are also stored with every run and printed in its report.

- The solve is only as good as the pairs. Few pairs, pairs bunched in one part of the image, or pairs near one plane leave the focal length and distortion poorly determined, and the stated uncertainties show it.
- Height by reverse projection measures the image feature, not stature (see above).
- Rolling shutter, motion blur, compression and interlacing in video frames are not modelled.
- A line of sight is tested against the scan only.

## References

- R. Hartley and A. Zisserman, *Multiple View Geometry in Computer Vision*, 2nd ed., Cambridge University Press, 2004: the normalised DLT, the decomposition of P, reprojection-error refinement.
- Z. Zhang, "A flexible new technique for camera calibration", *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 22(11), 2000, 1330–1334.
- D. C. Brown, "Decentering distortion of lenses", *Photogrammetric Engineering*, 32(3), 1966, 444–462: the radial and tangential distortion model.
- A. Criminisi, I. Reid and A. Zisserman, "Single view metrology", *International Journal of Computer Vision*, 40(2), 2000, 123–148.
- R. Drillis and R. Contini, *Body Segment Parameters*, Technical Report 1166.03, New York University, School of Engineering and Science, 1966; as reproduced in D. A. Winter, *Biomechanics and Motor Control of Human Movement*, 4th ed., Wiley, 2009, Fig. 4.1.
- R. T. Birge, "The calculation of errors by the method of least squares", *Physical Review*, 40, 1932, 207–227.
