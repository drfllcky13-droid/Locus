# 3D scenes

How the 3D scene is built from diagrams, how models are placed on the point cloud, and how the sun is positioned. Code: `app/src/scene3d/` (extrusion, roofs and the model library are pure functions with tests), `crates/locus-analysis/src/surface.rs` and `sun.rs` (pure, tested), commands in `src-tauri/src/scene3d_cmds.rs`, storage in `crates/locus-core/src/document.rs`.

## One coordinate frame

Diagrams, models and point clouds share the project frame: metres, right-handed, z up, with x east and y north as in the diagrams. Everything is stored in f64. The view draws relative to a render origin near the data (the point cloud's), and each position is converted to f32 only after the origin has been subtracted in f64. In a project frame with coordinates in the hundreds of thousands of metres, the round trip loses less than a micrometre (tested).

## The scene document

A 3D scene is stored like a diagram. Every saved state is an immutable revision with its SHA-256, written with an audit entry (`scene.created`, `scene.revised`) that counts its objects by kind. An extrusion or roof names the exact diagram revision (id and hash) it was built from, so the 3D can always be traced to the 2D it came from. It is not silently rebuilt when the diagram changes later: to pick up a newer diagram, the examiner adds a new extrusion.

## Extruding diagrams

Extrusion calls the same functions that draw the plan (`walls` and `roadLines` in `diagram2d/builders.ts`), so the 3D uses the plan's own coordinates. The alignment is exact by construction, not by fitting. Tests put every wall vertex on the plan's lines to 10⁻⁹ m, and check that every end of a plan wall face and jamb is a 3D vertex. The acceptance criterion is 1 mm.

- **Walls.** Each wall is a set of solid pieces between the inner and outer faces. Full-height pieces run between openings. Over a door the wall is kept above the head height (default 2.1 m). Over a window it is kept below the sill and above the head (defaults 0.9 m and 2.1 m). Corners are mitred as in the plan. The base elevation and wall height are set per extrusion. The base can be typed, or picked on the point cloud: a plane is fitted there, and the plane's height is used.
- **Floor slab** (optional): the walls' outer outline, a given thickness below the base.
- **Roads.** The lane surface lies between the outer edge lines, and the shoulders lie beyond them. Each painted line becomes a strip of the given width (default 0.1 m) centred on the plan's line, 3 mm above the surface so the two don't flicker. Broken lines use the road's own dash pattern, and curves follow the plan's arcs.

## Roofs

A roof sits on a room's outer wall outline, and belongs to the extrusion it was added to: its eaves height is measured above that extrusion's base, so moving the base moves the roof. Every sloped plane passes through the wall line at the eaves height, so the roof meets the walls exactly. An overhang continues the slope outward and down.

- **Flat:** any outline, grown by the overhang, a slab of the given thickness.
- **Shed, gable, hip:** rectangular outlines only (four corners square within 0.5°). The ridge of a gable or hip roof is at eaves + half the width × tan(pitch). The ridge of a hip roof is shorter than the building by its width (hips at 45° in plan). The gable end walls are included.
- A hip roof on any other shape needs a straight skeleton, which is not built. Such rooms get a flat roof.

## The model library

Every model is generated in code from its dimensions. Nothing is imported or copied, and nothing is branded. Model coordinates are metres, with the origin on the ground at the model's centre, +x forward and z up. Tests check that each model's bounding box matches its parameters.

- **Vehicles** by class: car, SUV, pickup, van, box truck, bus, motorcycle, bicycle. Each has an editable length, width, height and wheelbase. The body is a side profile of the class, extruded across the width, and sits on wheels at the wheelbase. These are generic shapes, not any make or model. For a specific vehicle, set its dimensions from its specification or a measurement.
- **People.** A jointed figure scaled so that standing it is exactly the given height. The pose is set by joint angles (torso lean, hips, knees, shoulders, elbows) with standing, sitting, kneeling and lying presets. The proportions are generic, so the figure shows posture and stature, not a particular body.
- **Furniture:** table, chair, sofa, bed, cabinet, shelving, desk, each sized by its box.
- **Weapons:** generic, unbranded handgun, long gun, knife and blunt object, sized by overall length and lying flat.
- **Evidence markers:** numbered tents of a given size.

## Placing models, and snapping to the point cloud

A new model is placed at the view's centre. It is moved by typed position and heading, or with the move and rotate gizmo. The stored placement is a 4 × 4 matrix to the project frame in f64.

**Snapping.** The examiner clicks the point cloud. The picked point is resolved again in Rust from the stored data, as for measurements. Every visible point within the snap radius (default 0.1 m) is gathered, and a plane is fitted to them by total least squares (`fit_plane`, as used for measurements). The model's base goes onto the plane, at the pick moved along the normal onto it. With "tilt to the surface", the model's up axis is turned onto the normal, keeping its heading. The normal is the one facing the camera, so it points out of the surface being looked at.

The fit's residual is shown and stored with the model: the number of points and the RMS and worst distance from the plane. At least 10 points are needed. A residual over 5 mm is flagged as not a flat surface. Tests: on a synthetic floor with ±1 mm noise, far from the origin, and on a tilted plane, the snapped base lies within 1 mm of the true plane and the normal within 0.5°. A wall seen from one side gets the normal facing that side.

Limitations: a snap puts the model's base plane on the surface at one point. It doesn't check that the rest of the model rests on the cloud (a car on a slope touches with its wheels, not its centre). Check the placement visually, or place the model by measured positions.

## Imported meshes

OBJ, glTF/GLB and PLY meshes imported as evidence can be shown in the 3D view (the Meshes list, off until ticked) and placed in a scene ("Add an imported mesh").

**Frame and unit.** A mesh is drawn in the project frame in metres: its own coordinates are multiplied by the unit recorded at import (glTF is metres by its specification; OBJ and PLY take the unit the examiner chose). glTF is y up and the project frame is z up, so glTF coordinates (x, y, z) become (x, −z, y), a +90° turn about x. This is the inverse of the turn the scene's glTF export applies, so an exported scene re-imported lands where it was (`fileToProject`, tested). OBJ and PLY are taken as z up, as scans are. In the 3D view a mesh sits at its own coordinates, like a scan with no registration; meshes are not registered.

**Placing.** A mesh added to a scene gets a pivot: the bottom centre of its bounds, stored with it. Its placement (position, heading, uniform scale, or the move and rotate gizmo) puts that pivot at the given point, so a mesh first added sits exactly where the 3D view shows it. The placement is stored as a 4 × 4 matrix in the scene revision, with the mesh's evidence id and SHA-256. Placed meshes are drawn in renders and animations like other static objects and are exported with the scene glTF. A scale other than 1 changes the drawn size of measured geometry; it is recorded in the revision and should be explained if used.

**Integrity.** The file is read from the project's evidence copy and its SHA-256 compared with the hash recorded at import before anything is drawn. A file that no longer matches is refused with a message naming both hashes. A file is checked when it is first shown in a session; a change made while the project stays open is found on the next open or by Verify evidence.

**Limitations.** Meshes can't be picked or measured: picks and measurements go through them to the point cloud. OBJ materials (a separate .mtl file) and glTF external buffers or textures are not part of the evidence item and are not loaded; OBJ and PLY are drawn in grey (PLY vertex colours are used). A .gltf with external buffers can't be shown; import the .glb. Vertices are drawn in single precision, so a mesh stored in large georeferenced coordinates loses precision (about 3 cm at 500 km).

## Materials and lights

Each part of an extrusion, roof or model has a physically based material: a preset (asphalt, concrete, plaster, brick, wood, painted metal, metal, glass, rubber, fabric, skin, road paint, marker, grass, roof tile) with approximate roughness and metalness, and a colour the examiner can change. Presets are for legibility, not photometric accuracy.

Lights: ambient, point, spot and directional, as many as needed, plus a soft fill light. Point and spot lights cast shadows.

## The sun

The sun's direction comes from the place (latitude, longitude) and time (UTC) by the NOAA solar position algorithm, after Meeus. It is implemented in `locus-analysis::sun` and gives azimuth, elevation, the elevation after standard atmospheric refraction, declination and the equation of time. NOAA states the algorithm is accurate to about one arcminute (0.0167°) for 1800–2100. Near the horizon, refraction depends on the weather, and the stated uncertainty grows with it (taken as a tenth of the refraction).

Tests: the NREL SPA worked example (Reda & Andreas, NREL/TP-560-34302: Golden, Colorado, 17 October 2003) is reproduced to within 0.02° in azimuth and 0.03° in zenith. The solstice declination (23.44°) and the equation of time's extremes (+16.4 min in early November, −14.2 min in mid-February) are reproduced. At solar noon the sun is due south at the expected height.

The scene stores the computed azimuth, apparent elevation and uncertainty with the place and time, so a saved revision records what was shown. The project frame's +y is not necessarily true north: the examiner sets the direction of true north in the project frame (degrees anticlockwise from +y). The sun's bearing in the frame is its azimuth minus that angle. Shadows use a 4096² shadow map over the built objects, so shadow edges are approximate at the scale of centimetres.

The sun here is for visualising light and shade. A question such as "was this window in direct sun at 14:10" should be answered from the computed azimuth and elevation and the measured geometry, not from the rendered shadows.
