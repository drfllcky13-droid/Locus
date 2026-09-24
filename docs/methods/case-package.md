# Case package

A case package is a folder, for a USB drive, that lets someone without Lotus view a case: a prosecutor, defence counsel, a court. Nothing needs installing and no administrator rights are needed. It can't change the case, and it proves it hasn't been changed.

## What's in it

| Path | What |
|---|---|
| `Lotus Viewer.exe` | The Lotus program itself. It opens as the read-only viewer when it finds the package's manifest beside it, so the viewer is exactly the validated program. |
| `case/project.sqlite` | A consistent copy of the case's database (SQLite `VACUUM INTO`): evidence records and hashes, registrations, diagrams, 3D scenes and animations, analyses, measurements, the audit log. |
| `case/derived/` | The point clouds (octrees) and cleanup bitmaps the viewer draws. |
| `case/evidence/` | The original evidence files, only if chosen when making the package. Their hashes are always in the database and the case report. |
| `reports/` | Printed when the package is made, from the saved records: the case report, every analysis report (a withdrawn record isn't reprinted, but the case report lists it with its reason), each diagram at the largest standard scale that fits A3 landscape (1:50 to 1:5000), and the applied registration's report. |
| `videos/` | The renders, each copied only if it still matches the SHA-256 it was logged with. |
| `README.txt`, `THIRD_PARTY_NOTICES.txt` | How to open it; licences. |
| `manifest.json` | Every file above, with its SHA-256 and size; the project, case number, who made the package and when, the app version, and the source project's state head. It also lists anything that couldn't be included (a render whose file had moved or changed; a diagram too big for A3 at 1:5000) and why. |

The **package hash** is the SHA-256 of `manifest.json`. Because the manifest lists every file's hash, the package hash covers them all. It is logged in the source project's audit log (`export.written`) when the package is made. Every file in the package is marked read-only.

## The viewer

- **On opening**, it re-hashes every file against the manifest, the viewer program included. It shows "All N files match the package manifest", or lists each file that is missing, changed or added. The package hash is on the side panel and under Help → About.
- **Read-only, three ways.**
  - The database is opened with SQLite's read-only flag, so nothing can write to it.
  - The project refuses every change before it gets that far, with "this is a read-only case package".
  - The editing tools aren't shown: import, cleanup, diagrams, the scene builder's editing, new analyses, exports.
  - The audit log is still verified on opening. Nothing is logged, since nothing can be.
- **What it does:**
  - navigate the point clouds, clipping and display settings;
  - measure: distance, angle, area and height, with the case's point uncertainty. These measurements last for the session only, are marked "viewer (not saved)", and can't be saved;
  - show the 3D scene as saved, and play its animation, with its time zero and the "look through" views;
  - open any report or video with the computer's own PDF reader or video player.

## Requirements and limits

- **Windows.** The viewer uses Microsoft Edge WebView2, which is part of Windows 11 and of up-to-date Windows 10. It isn't bundled: it is Microsoft's, and outside rule 7. On a machine without it, the viewer won't start.
- WebView2 keeps its browser cache in the user's local application data. That is the only thing written outside the package, and it holds no case data.
- The package is as large as the case's point clouds; they aren't thinned.
- Diagrams are included as scaled PDFs, not as editable drawings.

## Checked

- Unit tests: a read-only copy refuses writes (and so does straight SQL on it); the manifest catches a changed, missing or added file.
- In the app, on a project with a 1.57M-point scan, a measurement, a skid analysis, a diagram and an animated scene:
  - the package was made from the Export section (10 files);
  - its viewer was started from the package folder;
  - it reported all 10 files matching and showed the package hash on its panel and in About;
  - it refused four kinds of change (an analysis, the case number, deleting a measurement, a diagram);
  - it measured for the session (id −1, "viewer (not saved)") and played the animation;
  - afterwards every file still matched its manifest hash, no file had been added, and every file was still read-only.
