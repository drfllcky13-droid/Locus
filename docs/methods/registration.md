# Registration

Registration finds each scan's pose: the rigid transform that places its points, stored in the scan's own frame, into the project frame. Code: `crates/locus-register` (pure mathematics, no file access), commands in `src-tauri/src/register_cmds.rs`, storage in `crates/locus-core/src/registration.rs`.

Registration never changes evidence or octrees. A run stores its parameters, every link with its point pairs and statistics, and each scan's pose. Runs are immutable. Deleting or forcing links and re-solving makes a new run that names its parent. Applying a run makes the scene use its poses; reverting goes back to the poses stored in the files. Every run and every apply or revert is audit-logged.

## Inputs

Each scan's points are read from its octree in scan-local metres, without points removed by cleanup. Scans larger than 8 million points are thinned evenly by record number. Intensity is kept for checkerboard detection.

If the examiner chooses, the poses stored in the files are used as rough starting poses. Scanners write these from their on-site pre-registration, compass or inclinometer. They only narrow the search (see Cloud-to-cloud); the final poses come from the links.

## Targets

**Spheres.** Every point's surface normal comes from its 12 nearest neighbours (smallest principal axis, turned toward the scanner). Each point votes for a centre one nominal radius behind its surface. Points on a sphere of that radius vote for the same place; points on other surfaces spread their votes. Each vote peak is then fitted:

- a geometric least-squares sphere fit on the points near the peak, trimming outliers as the fit tightens;
- accepted if the fit with a free radius agrees with the nominal radius within 3 mm, the RMS residual is at most 5 mm, at least 30 points support it, and they cover a cap of at least 30°;
- the reported centre comes from the fit with the radius held at nominal, which is better conditioned when only part of the sphere is visible.

The centre's covariance comes from the fit residuals.

**Checkerboards (2 × 2).** Candidates are cells half a board wide whose points span a high intensity contrast (90th over 10th percentile at least 4). Around each candidate:

1. A plane is fitted robustly (RANSAC, then least squares), and its points are labelled dark or bright at the midpoint of the 10th and 90th intensity percentiles.
2. A coarse search finds the centre and rotation of the saddle pattern that best explains the labels inside a circle inscribed in the board.
3. Each point is paired with its nearest neighbour of the other label, and the midpoint of each pair is an edge sample. The two edge lines are fitted by least squares, and their intersection is the centre. This is repeated three times.
4. The board is accepted if at least 90 % of labels agree with the pattern, both lines are seen on both sides of the centre (which rules out the board's outer edge), and the lines are within 5° of perpendicular.

Precision is limited by point spacing: an edge sample can lie anywhere between its two points. The reported covariance therefore never uses less than spacing/√12 per sample, and counts only half the samples as independent.

**Matching targets between scans.** Rigid motion preserves distances. Two targets in one scan can only match two in another if their separations agree, within a tolerance of 5 mm plus three times the combined σ of the four positions, and their kinds match. Every consistent triple of matches proposes a transform; the one that brings the most targets into agreement wins and is refitted on all of them. If a clearly different transform explains as many targets (a symmetric layout), the match is reported as ambiguous and not used.

## Cloud-to-cloud

**Fine: point-to-plane ICP.** The source scan is downsampled to 5 cm voxels (keeping the real point nearest each voxel's centroid). Each source point is paired with its nearest target point within 15 cm and minimises its distance to that point's tangent plane.

- Huber weights, scaled by the median absolute residual, keep a few wrong pairs from pulling the result.
- The point-to-plane threshold shrinks from 50 cm to 2 cm as the fit tightens.
- ICP reports *overlap* (the share of source points paired in the final iteration) and *conditioning*: the weakest direction's eigenvalue per pair, with rotation expressed as motion at the paired points' RMS radius. Near zero means a motion the geometry barely resists, like sliding along a corridor.
- A cloud link is kept only if ICP converged with at least 20 % overlap and conditioning of at least 0.01.

**Coarse: starting pose from shape.** Scans must be levelled (tilt-compensated scanners hold the vertical to a fraction of a degree), so the rotation between two scans is a heading plus a small tilt.

1. Both scans are downsampled to 10 cm. The most distinctive 20 % of points get Fast Point Feature Histograms (Rusu et al., 2009; 1 m radius), and mutual best feature matches are kept.
2. For every heading in 0.5° steps, each match votes for the translation it implies. Up to 20 vote peaks per heading become candidates. With a rough pose, only headings within 10° and translations within 2 m of it are searched.
3. Candidates are screened on a sample, the best 20 are refined by ICP at voxel scale, and those are scored on all points. The score is the share of wall-like surfaces landing on a matching surface, minus the share of points placed where the other scanner saw straight through empty space (checked both ways).
4. If a clearly different candidate scores within 0.05 of the best, the result is ambiguous and not used.

## Survey control

Targets in a scan can be matched, in the same way, to surveyed coordinates entered with their precision. A control link ties that scan to the project frame. With control, the adjustment is in the control frame; without it, the first scan keeps the pose stored in its file and the others are placed relative to it.

## The adjustment (pose graph)

All links become point pairs, so targets, cloud-to-cloud and control combine in one least-squares adjustment (hybrid mode):

- a target link pairs each matched target, weighted by both detections' covariances;
- a control link pairs a target with its surveyed coordinates, weighted by detection and survey covariances;
- a cloud link is represented by 12 points spread over the overlap (farthest-point sampling), mapped through the ICP result, each with the precision the examiner sets (default 2 mm). ICP's own formal precision is far smaller than its real accuracy, because neighbouring points aren't independent and systematic effects aren't modelled, so it isn't used.

The adjustment minimises Σ rᵀWr over all pairs by Gauss–Newton on small rotations and translations of every scan, linearised near the scans so far-from-origin coordinates stay well conditioned.

**Testing each link.** After the adjustment, each link's χ² (Σ rᵀWr over its pairs) is compared with the 99.9 % point of χ² with 3 × pairs degrees of freedom (Wilson–Hilferty approximation). A link that fails is flagged, set aside, and the adjustment repeated, worst link first, until every remaining link passes. A forced link is never set aside. A link that is the only connection to part of the graph can't be checked and is reported as *untested*.

**Verified scans.** A scan is *verified* when trusted links connect it to the reference, i.e. links that are neither flagged nor shape-only. A cloud link is *shape-only* when its starting pose came from the coarse search with no rough pose and no targets. In a symmetric scene, every such link can agree on the same wrong answer, which no consistency test can catch. The examiner must check unverified scans against the scene.

## Accuracy on synthetic data

`crates/locus-register/tests/accept.rs` uses scenes from `crates/locus-synth`: a 40 m × 30 m room with pillars, crates, 20 spheres and 8 checkerboards, scanned from 6 levelled stations at 4 million points each, with 1 mm range noise. Rough poses are off by 0.5 m and 5°.

- The worst error relative to the first scan is 0.22 mm and 0.0008° (criterion: 2 mm and 0.02°).
- A cloud link injected 2 cm and 0.1° wrong is flagged (χ²/dof 252 against a limit of 1.89), and accuracy is kept.
- Without rough poses, shape matching alone placed two scans flipped 180° in this nearly symmetric room. They were reported unverified.

## Limitations

- Coarse cloud-to-cloud needs levelled scans. Others need targets, control or a rough pose.
- Symmetric or repetitive scenes (corridors, regular columns) can defeat shape matching. Such links are marked for review, but the examiner must look.
- Checkerboard centres are only as precise as point spacing allows; distant boards contribute little.
- A cloud link's precision is an examiner-set figure, not measured.
- Thinning to 8 million points per scan reduces density evenly, including on distant targets.
