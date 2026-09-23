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
- **Fold radius.** A strong barrel distortion polynomial stops increasing at some radius and folds back. Beyond that radius a point outside the field of view would land inside the image, so the model is only used inside it. The limit is where `1 + 3k1 r² + 5k2 r⁴ + 7k3 r⁶` reaches 0. The check is exact: that cubic in r² is tested at the point's radius and at its turning points before it. The generator had exactly this bug, found by the validation: a marker 67° off-axis appeared in the middle of the CCTV frame.

## Solving the camera

**Inputs.** Pairs of a pixel and the same point on the scan. Each scan point is resolved again from stored data, and its scan, record and cleanup revision are recorded.

- At least 6 pairs are needed, and more than half the lens model's unknowns.
- The photo is an image in the evidence, named by its record and hash.

**Lens models.** One of these is chosen automatically by leave-one-out (below), or by the examiner:

| Model | Unknowns besides the pose (6) |
|---|---|
| Focal length only | f |
| + k1 | f, k1 |
| + k1, k2 | f, k1, k2 |
| Full | f, cx, cy, k1, k2, k3, p1, p2 |

The simpler models hold the principal point at the image centre. Held there when it isn't (the generator's is 4–6 px off), they bias heights by about a centimetre; with few pairs, the full model's 14 unknowns are poorly determined instead.

**Choosing the lens model: leave-one-out cross-validation.** For each model the pairs can support (more than twice as many coordinates as unknowns with one pair left out):
- each pair is left out in turn;
- the camera is solved from the others (started from the solve on all pairs);
- the left-out pair is predicted, and its reprojection error recorded.

The model with the smallest RMS held-out error is chosen. A more complex model is chosen only when its held-out error is strictly smaller, so the full model is used only when it predicts pairs it wasn't fitted to better than the simpler ones. Every model's fitted and held-out RMS is reported, with the choice and the reason. The examiner can still set the model; the report then says it was their choice.

**Start, with points off one plane.** The direct linear transform on the pairs nearest the image centre, where distortion is least: at least 8 of them, or half, provided they are off one plane (Hartley and Zisserman 2004, §4.4 and §7.1).
- The pixels and points are normalised, and the smallest right singular vector gives P.
- P is decomposed into K, R and C. The RQ step uses the Cholesky factor of M Mᵀ through the exchange matrix.
- The sign is fixed so the points are in front of the camera, and a mirror-image result is refused.

**Start, with points on one plane** (smallest over largest eigenvalue of their scatter below 10⁻³: a CCTV that sees only the floor, say). This follows Zhang (2000) with the focal length as the only intrinsic unknown, and **assumes square pixels and the principal point at the image centre**; the report says so.
- The homography from plane coordinates to pixels (relative to the image centre) is found by the normalised DLT.
- Writing r = K⁻¹h with K = diag(f, f, 1), the constraints r1 ⟂ r2 and |r1| = |r2| each give f², and the valid values are averaged.
- If neither gives a valid f, the photo is nearly square on to the plane, and f is assumed to be the image's longer side (about a 53° field of view); the refinement then finds it, and the report says it was assumed.
- The rotation is completed with r3 = r1 × r2 and made orthonormal by SVD, and the translation comes from h3.
- The principal point stays at the centre in the refinement, and the full model is not available.

A subject's feet or head lying farther from the control points' plane than a quarter of their spread in it is warned about: its height rests on extrapolating a solve from one plane.

**Refinement.** Levenberg–Marquardt on the reprojection errors, each in units of its σ. That σ combines the pick σ (pixels) with the scan point's σ projected at its depth: `σ² = σ_px² + (f σ_point / z)²`.
- The refinement runs in stages: the pose and focal length, then k1, then k2, then the rest, as far as the chosen model goes. Each stage starts from the previous one, with the weights set twice per stage.
- Started directly with every parameter free from the DLT (which has no distortion), the CCTV case settled in a wrong minimum at 6 px RMS.

**Uncertainty: a parametric bootstrap, pooled over the plausible lens models** (Efron and Tibshirani 1993). For each lens model whose held-out error is within 25 % of the best (the models the pairs can't tell apart), the draws (1,000 by default, shared equally) are made like this:
- the camera's own projections of the scan points get fresh pixel noise of each pair's σ, inflated by the model's Birge ratio √(χ²/dof) when that exceeds 1 (Birge 1932);
- the camera is re-solved from them, starting from the solution.

Every σ of the camera comes from the pooled re-solves, and each re-solve is one draw of the heights' Monte Carlo (below).

The first version used the linearised covariance (JᵀJ)⁻¹. With few pairs it was too small: a camera 4.7σ off; the heights' 95 % intervals covered the truth about 90 % of the time. The bootstrap captures the nonlinearity of the distortion terms. Pooling captures the bias a simpler model brings when leave-one-out prefers it.

- Reported: position, heading/pitch/roll, focal length, principal point and distortion, each with 1σ; each pair's residual in pixels and in σ; RMS and χ².
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

**Uncertainty.** A Monte Carlo seeded so a run repeats exactly.
- Each draw takes one of the bootstrap's re-solved cameras and moves both image points by the pick σ.
- The 1σ and the 95 % interval (2.5th and 97.5th percentiles) are reported.

**Several frames.** A fixed camera's other frames (other images in the evidence) can be used for a subject: the camera solved on one photo applies to all of them. Apparent height changes from frame to frame with the phase of the gait (a walking person is shortest at mid-stride and tallest at mid-stance, by a few centimetres), and with footwear, headwear and posture. The report therefore recommends measuring each subject in several frames. For a subject measured in more than one frame it gives:
- the range of the heights across the frames;
- their mean and spread;
- the span of the frames' 95 % intervals.

The range is the result to report.

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

**What is measured.** The height of the top of what was marked, in that frame's posture and phase of the gait. Hair, headwear, footwear, a stride, a slouch or a head tilt all change it by centimetres, and none of these is corrected for. The report's limitations say so, and it recommends several frames.

## Photo over the 3D scene

The viewer can look through the solved camera: its pose, and a projection built from f, cx and cy with the photo fitted into the view. The photo is drawn over the scene with adjustable opacity.

- The photo is drawn through the lens model: a shader takes each screen pixel's undistorted image position, applies the distortion, and samples the photo there. Straight edges in the scan then line up with the photo's curved ones.
- In the photo editor, the scan's points (a sample of those loaded) are drawn over the photo through the solved camera, with each pair's residual drawn ten times longer.

## Witness perspective

The examiner states the eye position, as a floor point picked on the scan and an eye height above it (the witness's own or measured eye height as stated; no stature-to-eye-height ratio has been adopted, because none was found that is validated for this use), the point the witness was looking toward, and a horizontal field of view. The 3D view can be set to that eye.

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
| 40 seeds × 2 cameras × 3 people × 2 room layouts: 30 control markers (a camera sees 9–14, poor solves) and 60 (about 25, good solves); lens model by leave-one-out, 300-draw bootstrap | height mean error **6.4 mm** over 480 cases; with a good solve (height 1σ ≤ 10 mm, 398 of 480), 95 % of errors under **13 mm**, worst 28 mm; the 95 % interval covers the truth **94.6 %** of the time with few control points and **94.6 %** with many (95 % over all) |
| Lens models chosen by leave-one-out in those 160 solves | focal length only 2, + k1 44, + k1 k2 78, full 36 |
| Pairs on one plane (unit test) | camera exactly from the planar start (position to 10⁻⁶ m, focal length to 10⁻³ px) |
| In the app: 14 pairs clicked on the CCTV frame and the cloud, lens chosen by leave-one-out (+ k1 k2), three people's feet and heads clicked | camera 38 mm from the truth (1σ 36, 26, 17 mm); heights +4.8, +4.3 and +2.5 mm (1σ 11, 11, 9 mm) |

The bound, as approved: 95 % of errors under 2 cm when the solve states the height to 1 cm (1σ), and the stated 95 % interval covering the truth about 95 % of the time over all cases, poor solves included. Both are met: 13 mm, and 94.6 % for both poor and good solves. The validation prints the coverage for each.

## Assumptions

These are stored with every run and printed in its report.

- The image is a single central projection (a pinhole camera) with the distortion of the chosen lens model, square pixels and no skew.
- The scene has not changed between the image and the scan at the paired points.
- A subject's feet point is on the floor plane, and the top of the head is vertically above it.
- Frames other than the camera's photo come from the same fixed camera, not moved or zoomed.
- When the pairs lie on one plane: square pixels and the principal point at the image centre.

## Limitations

These are also stored with every run and printed in its report.

- The solve is only as good as the pairs. Few pairs, pairs bunched in one part of the image, or pairs near one plane leave the focal length and distortion poorly determined, and the stated uncertainties show it.
- Height by reverse projection measures the image feature in one frame, not stature: apparent height varies with the phase of the gait, footwear, headwear and posture. Measure in several frames and report the range.
- Rolling shutter, motion blur, compression and interlacing in video frames are not modelled.
- A line of sight is tested against the scan only.

## References

- R. Hartley and A. Zisserman, *Multiple View Geometry in Computer Vision*, 2nd ed., Cambridge University Press, 2004: the normalised DLT, the decomposition of P, reprojection-error refinement.
- Z. Zhang, "A flexible new technique for camera calibration", *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 22(11), 2000, 1330–1334.
- D. C. Brown, "Decentering distortion of lenses", *Photogrammetric Engineering*, 32(3), 1966, 444–462: the radial and tangential distortion model.
- A. Criminisi, I. Reid and A. Zisserman, "Single view metrology", *International Journal of Computer Vision*, 40(2), 2000, 123–148.
- R. Drillis and R. Contini, *Body Segment Parameters*, Technical Report 1166.03, New York University, School of Engineering and Science, 1966; as reproduced in D. A. Winter, *Biomechanics and Motor Control of Human Movement*, 4th ed., Wiley, 2009, Fig. 4.1.
- B. Efron and R. J. Tibshirani, *An Introduction to the Bootstrap*, Chapman & Hall, 1993: the parametric bootstrap.
- M. Stone, "Cross-validatory choice and assessment of statistical predictions", *Journal of the Royal Statistical Society B*, 36(2), 1974, 111–147: leave-one-out cross-validation.
- R. T. Birge, "The calculation of errors by the method of least squares", *Physical Review*, 40, 1932, 207–227.
