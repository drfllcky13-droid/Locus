# Photogrammetry

Lotus turns photos or a video into a scaled point cloud by running COLMAP, installed separately by the examiner's agency. It then scales and places the result and imports it as evidence.

- Code: `crates/locus-photo` (runner, camera models, scaling, EXIF, video), `src-tauri/src/photo_cmds.rs` (commands), `crates/locus-report/src/photo.rs` (report).
- Why COLMAP isn't bundled: `docs/phase8-colmap-licence-review.txt`.

## COLMAP setup

In the panel's setup section, the examiner chooses COLMAP's `COLMAP.bat` (its release folder) or `bin\colmap.exe`. Lotus:
- runs it to read its banner, which gives the version and whether it has CUDA;
- hashes the executable;
- refuses versions older than 3.9.

Copies found on the PATH or in the usual install folders are offered.

The setup is the machine's, kept in the app's config folder. Each run records the COLMAP path, banner and executable SHA-256 in its analysis record and report.

**CPU-only feature extraction** runs COLMAP's own SIFT (from VLFeat, BSD) for extraction and matching. That avoids SiftGPU, whose licence is non-commercial. It is slower, and it doesn't change the method.

The setup help summarises COLMAP's component licences so the agency can decide for itself.

## Reconstruction

Inputs are either:
- photos already in the project as evidence (hashed, read-only), hard-linked or copied into the run's folder;
- a video in the project as evidence, sampled at a chosen interval.

**Video frames** are sampled through Windows Media Foundation, part of the operating system, so nothing is shipped:
- They are decoded in order. A frame is kept at the first timestamp at or after each multiple of the interval, counted from the first frame.
- Each is written as a PNG.
- Frames are matched sequentially.
- On macOS and Linux, video import is refused with a message (Blocked).

**The stages**, each a separate COLMAP process, with its command line, exit code and duration recorded:
1. feature extraction;
2. exhaustive matching (photos) or sequential matching (video);
3. incremental mapping;
4. conversion of each sparse model to text;
5. with CUDA and when asked for: undistortion, PatchMatch stereo with geometric consistency, and fusion into a dense point cloud.

The largest sparse model is kept. Smaller disconnected ones are reported.

Progress is read from COLMAP's log, and Cancel stops the process. Crashes with Windows' out-of-memory or fail-fast codes suggest a smaller image size.

**Settings:**
- Camera model: SIMPLE_RADIAL, OPENCV or OPENCV_FISHEYE.
- One camera for all photos: the same body, lens and fixed zoom. Always on for video.
- Dense or not.
- The longest image side for features: 4800 px by default, more than COLMAP's 3200 (see Validation). COLMAP 4.2's CPU SIFT crashed at the benchmark's full 6048 px.
- The longest image side for the dense cloud: 2000 px by default.
- The horizontal field of view, when the images carry no focal length (video frames, photos without EXIF).
  - It sets COLMAP's starting focal length, with the principal point at the centre and no distortion. COLMAP takes that as a prior.
  - For fisheye models f = (w/2) / (fov/2 in radians); for the others f = (w/2) / tan(fov/2).
  - A fisheye video needs it. COLMAP can't recover a fisheye focal length from matches alone and marks every pair degenerate. That was found on the first in-app video run.
  - A camera's specified field of view is enough: the prior is refined in bundle adjustment. The test gave 100° where the truth is 101°.

## Scaling and placing

A reconstruction has arbitrary scale, position and orientation. One of three methods fixes it. Each target is clicked in two or more registered photos and triangulated with the reconstruction's own cameras: least squares over the rays. Its ray angle and per-photo reprojection error are shown.

- **Known distances** (scale only).
  - The scale is the weighted mean of true / model length, weighted by (model length / σ)².
  - Its 1σ is the larger of what the tolerances give and the spread between the distances.
  - The model is levelled by the photos' mean up direction (each camera's −y). This is approximate: tilted photos tilt it.
  - The model is centred on its points. Its position and heading are arbitrary; register it to a scan to relate it to other evidence.
  - Warnings: distances disagreeing by more than 1 %, and an end seen from photos less than 5° apart.
- **Control points** (similarity: scale, rotation, translation; Umeyama).
  - Each point's coordinates are typed, or picked on a scan already in the project, which places the reconstruction in that scan's frame.
  - Check points are held out and their errors reported. The report warns when there are none.
- **The photos' GPS** (similarity from the camera centres to a local east-north-up frame, origin at the first photo).
  - Positions and DJI RTK tags are read from EXIF and XMP by a small in-house reader.
  - The report warns unless every photo has an RTK fixed solution, because ordinary GPS is good to metres.

The scale's relative 1σ for control points and GPS is RMS / √(Σ |x − x̄|²).

## Checks

Both known distances and control points can be marked as checks, held out of the scaling. For each check, the report lists what the scaled reconstruction measures, the known value and the residual.

A check is flagged, with a warning, when its residual exceeds its 95 % limit:
- **A check distance:** 2 × √(σ_known² + σ_model(length)²). σ_model here uses the benchmark's terms with the scaling's own uncertainty, not the case's checks, so the limit doesn't depend on the check itself.
- **A check point**, in 3-D: 2.80 × σ per axis, the 95 % point of χ² with 3 degrees of freedom. σ per axis is √(σ_coordinates² + (6 mm/√2)² + (p × r)²):
  - σ_coordinates: the point's stated 1σ (5 mm by default; for a point picked on a scan, the project's point uncertainty);
  - p: the larger of the benchmark percentage and the scale's relative σ;
  - r: the point's distance from the control points' centre.

The report warns when there are no checks.

## Measurement uncertainty

Every measurement made on a photogrammetric point cloud takes the uncertainty of its run:

σ(length) = max(p × length, floor), 1σ

This keeps short distances from getting unrealistically tight bounds, which the scan's per-point σ alone would give them.

- **p** is the larger of:
  - the benchmark's 0.35 %;
  - the case's check distances' RMS relative residual;

  combined in quadrature with the scaling's own relative σ.
- **floor** is the larger of:
  - the benchmark's 6 mm;
  - the case's check distances' RMS residual (for check points, their RMS 3-D error × √(2/3), a distance's share of it).

**Benchmark values** (ETH3D pipes, four runs):
- The RMS relative error of distances of 0.5 m and more was 0.23–0.33 %, so p is 0.35 %.
- A point's error, with the scaled model placed on the truth, was 3.0–3.8 mm per axis. A distance between two points is √2 × that: 6 mm.

The benchmark's DSLR photos were taken 1–3 m from the surfaces. Scenes photographed from further away, or with poorer cameras, have larger absolute errors. That is why checks matter: when the case's own checks show more, they set the terms. The report says which set them.

**Other kinds of measurement:**
- A height takes the same rule as a length.
- An angle takes the floor across each arm: floor × √(1/a² + 1/b²) rad, for arms a and b.
- An area takes max(2p × area, floor × perimeter / 2).

A measurement's 1σ is never lowered below what the scan's point σ gives. The measurement's record names the run, and the UI shows the terms.

The model is fixed when the cloud is imported, and stored in the run's record.
- Runs stored before it existed read as the benchmark's terms.
- Withdrawing a run doesn't remove its cloud from evidence, so its model still applies.

## Import and provenance

The scaled points (COLMAP's dense cloud when made, else the sparse points) are written as E57: double-precision metres, with colour. The file is imported as evidence with its own hash, through the same path as any file.

The run is stored as an audit-logged analysis record. It holds:
- the inputs' evidence IDs and hashes (for video, the frames' names and times);
- COLMAP's path, banner and executable hash;
- the settings and every stage's command line;
- the reconstruction statistics;
- the scaling with every target and residual;
- the output evidence's ID and hash.

The PDF report prints all of it.

## Validation

`crates/locus-photo/tests/eth3d.rs` (heavy; data outside the repository, see its README) uses **ETH3D's "pipes"** scene: 14 DSLR photos with a fisheye lens, and ground-truth camera poses and calibration registered to a laser scan. The benchmark data is CC BY-NC-SA 4.0 and is used only as test input.

1. COLMAP reconstructs from the photos alone.
2. Each reconstructed point is triangulated again with the ground-truth cameras, from its own observations. Their median reprojection error is 0.5 px, a check on Lotus's camera models.
3. The model is scaled by 3–4 known distances of 2–3 m between well-triangulated points.
4. Every other distance of at least 0.5 m is compared with the truth.

**Result** (COLMAP 4.2.0, CPU features, 4800 px; COLMAP's result varies a little from run to run):
- 14 of 14 images placed;
- about 95,000 distances per run;
- over five runs: median 0.16–0.22 %, 99th percentile 0.51–0.83 %, worst 0.97–1.43 %;
- 99.86–100.00 % within 1 %.

COLMAP's default 3200 px left the 99th percentile at 1.02 %, hence the 4800 px default. GPU SIFT at 3200 px: 99th percentile 1.20 %.

The acceptance bound: 99 % of distances within 1 %.

**Camera-centre distances** are much less precise, about 20 mm absolute: 2 % over 0.5 m baselines. Measure between scene points, not camera positions.

**Other checks:**
- Unit tests cover:
  - each camera model's projection and its inverse;
  - triangulation;
  - the three scaling methods on a synthetic model;
  - the text-model and PLY readers;
  - EXIF GPS and RTK tags;
  - geodetic to ENU;
  - video sampling (a 4 s, 25 fps test clip made with ffmpeg only for the test: 8 frames at 0.5 s).
- **In the app** (the pipes photos imported as evidence; setup and run through the panel, with CPU features and dense at 2000 px):
  - scaled by two known distances between ground-truth points, clicked at their true pixels (residuals 0.8 and 0.9 mm);
  - 416 distances between 33 other ground-truth points triangulated in the reconstruction: median 0.21 %, worst 0.45 %;
  - the 294,005-point dense cloud imported as evidence and the report printed.
- **In the app, from a video** (the 14 photos at half size as a 2 fps H.264 MP4, written with Media Foundation by `locus-validate gen-video`; OPENCV_FISHEYE with a 100° field of view):
  - 10 of 14 frames placed; the other 4 formed a separate group, warned in the report;
  - scaled by six ground-truth points picked on ETH3D's laser scan, imported as evidence (four control, two check):
    - picks 1–4 mm from the true points;
    - checks 3.4 and 3.2 mm, against limits of 17 and 18 mm;
  - 95 other ground-truth distances: median 0.09 %, worst 0.38 %;
  - the cloud imported and the report printed.
- **Dense image size.** The dense cloud uses its own size, 2000 px by default. PatchMatch at the 4800 px feature size took 26 minutes for 14 photos, against about 8 minutes at 2000 px.

## References

- J. L. Schönberger and J.-M. Frahm, "Structure-from-Motion Revisited", CVPR 2016.
- J. L. Schönberger, E. Zheng, M. Pollefeys and J.-M. Frahm, "Pixelwise View Selection for Unstructured Multi-View Stereo", ECCV 2016.
- S. Umeyama, "Least-squares estimation of transformation parameters between two point patterns", IEEE TPAMI 13(4), 1991.
- T. Schöps et al., "A Multi-View Stereo Benchmark with High-Resolution Images and Multi-Camera Videos", CVPR 2017 (ETH3D).
- NIMA TR8350.2, Department of Defense World Geodetic System 1984 (the WGS84 constants).
- CIPA DC-008, Exif 2.32: the GPS tags.
