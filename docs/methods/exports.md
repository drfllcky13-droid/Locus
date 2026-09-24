# Reports and exports

Everything Locus writes out of a project comes from saved records, never from unsaved work in the view. Every file is hashed (SHA-256) and logged in the audit log with what it was made from: `analysis.reported`, `diagram.exported`, `report.exported` or `export.written`. Those entries record reading the project, not changing it, so they don't move the project's *state head* (below).

## Reports

- **Analysis reports** (every tool), the **registration report**, the **diagram print**, the **time–distance–speed report** and the **case report** are Typst documents compiled in-process, with no clock inside: the time printed is passed in.
- **Reproducible.** The same project printed twice gives the same PDF except for the printed time. The tests build every report type twice and compare the bytes, then change only the print time and compare the text. In the app, two prints of a record differ in that one line.
- **Traceable.** Each report names:
  - the audit entry that recorded what it reports (sequence number and hash), whose details carry the record's SHA-256;
  - the project's **state head**: the newest entry that changed the project, excluding prints, exports and integrity checks. A report printed twice from the same project names the same head, and a changed project names a new one.
- **The case report** lists:
  - the evidence with each file's SHA-256, size, format, unit, import time and import entry;
  - the latest integrity check, with failures in "Needs attention";
  - every registration, diagram and 3D scene revision, analysis (withdrawn ones with their reason and time) and measurement, each with its hash and audit entry;
  - how to check it.

## Exports

| What | Format | Notes |
|---|---|---|
| Diagram | PDF | At the chosen scale and paper, with a 100 mm calibration bar (Phase 4). |
| Diagram | PNG, TIFF | The same printed page rasterised at 150, 300 or 600 dpi. The resolution is written in the file (PNG pHYs; TIFF X/YResolution in inches), so it prints at scale. The TIFF is baseline, uncompressed 8-bit RGB. |
| Diagram | DXF (R12, ASCII) | World metres in the project frame, one DXF layer per visible diagram layer. It covers lines, polylines, arcs, built rooms and roads (their stored geometry), dimensions (with the length as printed, to 1 mm), text, markers, points, north arrows and scale bars. A symbol is written as a circle of its size with its name, and underlay images are left out. Coordinates are written to 1 µm; text is ASCII. |
| Point clouds | E57 | The chosen scans' visible points (cleanup applied) in the project frame, as one scan with an identity pose. Coordinates are double precision, with colour and intensity. Streamed node by node, so a large cloud never has to be in memory. |
| Point clouds | LAS 1.2, LAZ | Point format 2 (colour), at 0.1 mm coordinate steps about the scene's centre. No coordinate reference system is written, because the coordinates are the project frame's. LAZ when the file name ends in .laz. |
| 3D scene | glTF binary (.glb) | What the scene builder builds (extrusions, roofs, models, lights), not the point cloud. glTF is y-up and the project frame is z-up, so the scene is turned −90° about x. The root node's extras record the units, the frame and the offset the geometry is relative to (float32 in glTF, so large coordinates stay precise). |
| Measurements | CSV (UTF-8) | One row each: the value and 1σ in their unit (m, °, m²), the point σ, the photogrammetry analysis if on one, the points as stored, who made it and when, and the audit entry. |
| Animation | MP4 | See [animation.md](animation.md). |

The log entry for a point cloud records the scans, the point count, the frame, the registration in use and that cleanup was applied. A diagram's entry records its revision and hash, the scale and the resolution; a 3D scene's records its revision, its hash and the origin.

## Limits

- DXF R12 has no units field that every reader honours. The header's `$INSUNITS` says metres, and so does a comment at the top of the file.
- A LAS or LAZ file has no coordinate reference system: a georeferenced project's coordinates stay in the project frame.
- Symbols in the DXF are placeholders (a circle and the symbol's name), not the drawn symbol.
