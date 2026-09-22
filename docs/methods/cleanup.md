# Point-cloud cleanup

Box delete, lasso delete, statistical outlier removal and voxel downsampling. Code: `crates/locus-octree/src/cleanup.rs`.

## What cleanup does and does not do

Cleanup never changes evidence or the octree built from it. Each operation records:

- its full parameters in the project database (`cleanup_ops`), audit-logged;
- for each scan, the set of source record numbers it removes, written as a bitmap under `derived/cleanup/` with the bitmap's SHA-256 stored alongside the operation.

The points hidden in the view are the union of all *active* operations. Undo switches an operation off and redo switches it back on. Both are logged, and nothing is deleted. On load, every bitmap is re-hashed and compared with its recorded hash, and a mismatch stops the project's point clouds from loading. The parameters are enough to recompute any operation from the evidence.

Measurements always resolve against visible points, and a pick made on a view drawn before a cleanup changed is refused (scan revision check).

## Operations

- **Box delete** removes every point inside an axis-aligned box in the project frame (the clip box).
- **Lasso delete** removes the points inside a polygon drawn on screen, in one of two modes. Before confirming, the examiner sees how many points each mode would remove.
  - **Visible surface only** (default): the view is divided into cells of 3 × 3 pixels, and each cell's nearest point sets its front depth. A point inside the lasso is removed only if its view depth is within 2 cm + 0.5 % of that depth of the front, so points on surfaces further back survive. The front is computed from every visible point in the cells the lasso touches, including points just outside it, so a lasso edge can't expose what is behind.
  - **All depths**: every point whose projection falls inside the polygon, including points hidden behind others.

  In both modes, points hidden by the clip box or clip plane are never touched. The stored parameters are everything needed to recompute the result: the polygon in normalized device coordinates, the camera's view-projection matrix (f64), the render origin, the mode with its viewport size, cell size and tolerances, and the clip box and plane.
- **Statistical outlier removal** (Rusu et al., 2008): for each point, the mean distance to its *k* nearest neighbours. A point is removed when its mean exceeds the scan-wide mean of those means by more than *n* standard deviations. Parameters: *k* (default 8) and *n* (default 2). It runs per scan, and only on points inside the clip box when that is on.
- **Voxel downsample** keeps one point per voxel of edge *s*: the point nearest the voxel centre, with ties going to the lower record number. Voxels are aligned to the scan's own origin. The operation is deterministic.

## Limitations

- Outlier removal and downsampling work tile by tile, to bound memory. Each tile reads a margin of 5 % of its edge for neighbours. A point whose *k* neighbours lie beyond that margin at a tile border gets a slightly overestimated mean distance. This matters only for sparse data near tile borders.
- Statistics are per scan, so a dense scan and a sparse scan are judged against their own distributions, not a common one.
- The visible-surface test uses the full-resolution data, not what the view happened to draw at its current level of detail. Where a surface is sparser than one point per 3 × 3-pixel cell, points behind it show through the gaps and count as visible. Zoom in, or use a clip box, when that matters.
- Surfaces closer together than the tolerance (2 cm + 0.5 % of distance) are treated as one surface.
