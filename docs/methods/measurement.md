# Point-cloud measurements

Distance, angle, polygon area and height above a plane, measured between points picked in a scan. Code: `crates/locus-analysis/src/measure.rs`.

## Where the coordinates come from

Clicking in the 3D view only *identifies* a point: the GPU pick pass returns which node and which point in it. The coordinates are then read in Rust from the octree's stored f64 values. Those values are the source file's coordinates multiplied by the unit factor, then transformed by the scan's pose, all in f64. The GPU's 32-bit floats are used only to draw and never feed a measurement. Every measured point records its scan and its record number in the source file, so it can be traced back to the evidence.

## Formulas

Points **p** are in meters in the project frame.

- **Distance** is ‖**b** − **a**‖.
- **Angle** at vertex **v** between rays to **a** and **c** is θ = atan2(‖**u** × **w**‖, **u** · **w**), where **u** = **a** − **v** and **w** = **c** − **v**. This is stable for all angles from 0 to π.
- **Plane fit** is total least squares: the plane passes through the centroid, and its normal is the eigenvector of the scatter matrix with the smallest eigenvalue (Jacobi method). The normal is oriented with z ≥ 0. The fit reports RMS and maximum perpendicular residual.
- **Polygon area** uses Newell's method: the area vector ½ Σ **pᵢ** × **pᵢ₊₁**, projected onto the fitted plane's normal. That gives the area of the polygon projected onto its best-fit plane. The plane residuals show how far the picked outline is from flat.
- **Height above plane** is (**p** − **c**) · **n** for the plane fitted to the chosen reference points. It is signed and positive on the side the normal points to (up).

## Uncertainty

Each picked point is assumed to have independent, isotropic positional uncertainty σ_p (1σ). σ_p is a project setting, default **2 mm**, a typical single-point range noise for terrestrial laser scanners at short range. The examiner should set it from the scanner's specification or a site test. Every result shows the σ_p it assumed.

Uncertainty is propagated to first order: σ² = σ_p² Σ (∂f/∂xᵢ)² over all coordinates of all points involved, with derivatives by central differences. For height above a plane, this includes the uncertainty of the fitted plane itself. Tests check the propagation against results derived by hand:

- distance: σ = √2 σ_p;
- right angle with unit arms: σ_θ = 2σ_p;
- height above the centroid of an n-point plane: σ = σ_p √(1 + 1/n).

## Assumptions and limitations

- Point errors are treated as independent and isotropic. Real scanner error is larger along the beam than across it and is correlated within a scan. Registration error between scans (Phase 3) is not included yet.
- Picking selects an existing scanned point, not a point interpolated on a surface. A measured corner is the nearest scanned point to the corner, and it can be off by up to about the point spacing at that spot. The UI loads full-resolution points around the cursor while a measurement tool is active to keep that spacing small.
- Polygon area assumes a simple polygon (edges don't cross) and is the area projected onto one plane. It is not the area of a curved surface.
- First-order propagation is poor when a result is nearly singular, for example an angle whose arms are almost zero length or a plane through almost collinear points. Those cases are refused.
