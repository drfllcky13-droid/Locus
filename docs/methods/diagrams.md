# Diagrams

Scale drawings of a scene in plan view: what is stored, how the geometry is built, how underlays are placed, and how a drawing is printed to scale. Editor code: `app/src/diagram2d/` (geometry, builders and underlay placement are pure functions with tests). Storage: `crates/locus-core/src/diagram.rs`. Printing: `crates/locus-report/src/diagram.rs` and `templates/diagram.typ`. Hand measurements have their own note (`hand-measurements.md`).

## The document

A diagram is a JSON document in project coordinates: metres, x east, y north, angles in radians anticlockwise from +x. It holds layers (name, colour, visible, locked) and entities, each with a stable id and a layer. Units are converted only for display. Text heights are the exception: they are millimetres on paper, so labels stay readable at any scale.

Every saved state is a new, immutable revision: the full document with its SHA-256, written in the same database transaction as its audit entry (`diagram.created`, `diagram.revised`). Nothing is overwritten. The editor saves 1.5 s after edits stop and when it closes; saving an unchanged document writes nothing. Undo and redo work within the editor and produce new revisions, not deletions.

## Drawing

**Snapping.** A click lands on the nearest candidate within 10 screen pixels, taking candidates in this order: endpoints (line ends, polyline and built-item vertices, arc ends and centres, placed points), midpoints of segments, the foot of the perpendicular from the previous click onto a segment, then the grid (half the visible grid spacing). The marker shows which kind of snap was used. Snapping can be switched off per kind. Positions typed into a form are used exactly.

**Dimensions** show the distance between their two points, computed from the coordinates, drawn offset to the side the examiner picks.

**Evidence markers** are numbered in the order they are placed. Renumbering in reading order (top to bottom, then left to right) happens only when the examiner asks for it, so numbers never change silently. **The legend** is recomputed from the document every time it is drawn: symbols in use (with counts), markers, and reference and measured points.

## Room builder

The examiner clicks the inside corners of the room. The outline is made anticlockwise if it was drawn the other way. Each wall's outer face is the inner face moved outward by the wall thickness, and neighbouring outer faces meet at a mitre (the intersection of the two offset lines).

Openings are placed on a wall (numbered from the first corner clicked), at a distance from the wall's start measured along the inner face, with a width:

- both faces are cut over the opening's width, and jambs are drawn across the wall at each side;
- a **window** gets a pane line along the middle of the wall;
- a **door** gets its leaf drawn open at 90° into the room from the hinge side, and a quarter-circle swing from the closed position to the open leaf, of radius equal to the door width.

An opening that does not fit on its wall is not drawn.

The room keeps its parameters (outline, thickness, openings) and the lines and arcs built from them. Changing a parameter rebuilds the geometry. The saved revision therefore holds exactly the lines the editor shows and the PDF prints, and the printer never rebuilds anything.

## Roadway builder

The examiner clicks points along the centreline and sets the number of lanes on each side, the lane width, the shoulder width, the centre marking (broken, solid, double solid or none) and a curve radius.

- **Curves.** With a radius set, every corner of the centreline is replaced by a circular arc tangent to both edges, drawn as chords of at most 2°. The chord sag is r·(1 − cos 1°), 1.5 mm for a 10 m radius. If an edge is too short for the full radius, the corner gets the largest arc whose tangent points lie within half of each edge.
- **Lines.** Every other line is the centreline moved sideways: lane dividers at whole multiples of the lane width, the edge line after the last lane, and the shoulder edge beyond that. Joins are mitred. On a curved centreline the offsets are concentric arcs: radius r − d on the inside of the curve, r + d on the outside.
- **Broken markings** are drawn as the painted pieces themselves, 3 m of line and 9 m of gap. The pattern carries on around corners. A double centre line is two lines 0.1 m either side of the centreline.

Limitations: the pattern and widths are typical values, not a survey of the real markings. Where a real road differs, draw the actual lines, or place them from measurements. A mitred join at a very sharp corner reaches far past the corner; round such corners with the curve radius.

## Underlays

An underlay is an image under the drawing. It can be a JPEG or PNG from the evidence store (for example an aerial photograph), or a slice of the point cloud written by the app. The diagram records the image's project file and SHA-256. The file is re-hashed every time it is shown or printed, and a mismatch is refused. The image's placement is a similarity: a uniform scale (metres per pixel), a rotation and a translation. It is stored with the underlay.

**Calibrating an image from known points.** The examiner clicks features on the image and gives each one's project position, by typing it or by choosing a reference or measured point in the drawing. With image pixels written as z = u − i·v (image rows run down) and project positions as w = x + i·y, the placement is w = a·z + b. This is linear in the complex numbers a and b, so the least-squares fit is closed-form:

    a = Σ (wₖ − w̄)·conj(zₖ − z̄) / Σ |zₖ − z̄|²,   b = w̄ − a·z̄

The scale is |a| and the rotation is arg a. Each point's residual is the distance between its given position and where the fitted image puts it. The residuals and their RMS are shown while the points are entered. When the calibration is applied, the points, placement and residuals are written to the audit log (`diagram.underlay_calibrated`) and stored with the underlay.

- Two points fix the placement exactly, so their residuals are zero: **two points give no check**. The editor says so. Use three or more points to see how well the image fits. A large residual on one point usually means a wrong click or a wrong position.
- A photograph is not a map. A similarity cannot remove perspective, lens distortion or relief displacement, and those show up as residuals. Use an orthophoto where accuracy matters, and treat the underlay as a guide to be drawn over. Measurements come from the drawing's own points, not the image.

**Point-cloud slice.** Every visible point (registered poses applied, cleanup removals excluded) with height between the two chosen values is binned into square cells of the chosen resolution. The cells are aligned to whole multiples of the resolution in project coordinates. The raster's top-left corner is therefore exactly (x₀·r, (y₁ + 1)·r) for the smallest cell column x₀ and largest cell row y₁, and the slice needs no calibration.

- A cell's colour is the mean colour of its points. If the scan has no colour, it is the mean intensity scaled by the brightest cell. If there is neither, the cell is dark grey. Empty cells are transparent.
- Rasters over 4096 × 4096 cells are refused; choose a coarser resolution.
- The PNG is written under `derived/slices/` and hashed. Its heights, resolution, placement, point count and the applied registration are written to the audit log (`diagram.underlay_sliced`).

Tests: cells land on the project grid with north at the top row, bad requests are refused (`crates/locus-octree/src/slice.rs`), and calibration recovers a known placement exactly from two points and reports a planted error (`underlay.test.ts`).

## Printing to scale

A diagram is printed at 1:n on A4 or A3, in portrait or landscape, through the same in-process Typst pipeline as the reports: no network, fonts bundled, warnings treated as errors. The newest saved revision is printed. Its number and hash appear in the title block, and the PDF's own hash goes to the audit log (`diagram.exported`).

- **Placement.** Page millimetres = project metres × 1000 / n, with the drawing centred in the frame and north up. Arcs are drawn as chords whose sag is under 0.01 mm on paper.
- **Fit.** The extent comes from every visible entity except underlays (an aerial photo is usually far larger than the drawing), plus 10 mm on each side for labels. If the drawing does not fit the frame at that scale, printing is refused and the message gives both sizes. Nothing is ever shrunk to fit. Underlays are clipped to the frame. A diagram with nothing but underlays is sized by them.
- **Underlay opacity** is printed by laying white over the image at 1 − opacity.
- **Checking a print.** The title block carries the scale statement, "Print at actual size (100 %)", and a calibration bar exactly 100 mm long with ticks every 10 mm. Measure the bar with a ruler: if it is not 100 mm, the printer scaled the page and the drawing is not at the stated scale.

Tests (`crates/locus-report/src/diagram.rs`): a 10 m line is 100 mm at 1:100 and 200 mm at 1:50; on the laid-out page (Typst's frame tree) the line measures 100.000 mm, the 5 m scale bar 50 mm and the calibration bar 100 × 2 mm; a drawing too large for the sheet is refused; built items and underlays land where their stored geometry and placement say; every symbol prints.

A physical check (printing at 1:100 and measuring with a ruler) is still to be done before release; see `docs/PROGRESS.md`, Blocked.
