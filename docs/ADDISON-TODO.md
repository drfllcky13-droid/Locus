# Addison's to-do list

Things only you can do. Nothing here blocks the next phases, but each is needed before release unless noted. Tick them off as you go.

## Checks with your hands or hardware
- [ ] **Case package on a clean PC.** Make a case package (3D view → Export → Make a case package…), copy it to a USB drive, and open `Locus Viewer.exe` on a Windows PC that has never had Locus, logged in as a standard user (no admin). Check that it opens, says all files match, and shows the package hash under Help → About.
- [ ] **1:100 print check.** Print a diagram PDF at actual size (no "fit to page"), then measure the 10 m line (should be 100 mm) and the 100 mm calibration bar with a ruler.
- [ ] **Mid-range GPU.** Run Locus on a PC with a mid-range graphics card (for example an RTX 3060 or GTX 1660), open a large scan, and tell me roughly how smooth navigation is (or run Help → About's benchmark if I add one).

## Decisions and information
- [ ] **Textbook worked examples** from Fricke (Northwestern): type the inputs and published answers for 3–5 examples each.
- [x] **Publisher domain:** `io.github.drfllcky13-droid.lotus` (2026-09-24).
- [x] **Angle convention:** no standard, keep both printed; floor-stain plan-view convergence is used (2026-09-24).
- [x] **Final product name:** Lotus (2026-09-24). In-house only for now.
- [x] **Physical validation studies:** your unit runs them (2026-09-24).
- [x] **Evidence.com:** not pursued (2026-09-24).
- [x] **GitHub Actions minutes:** stay free (2026-09-24).

## Before any commercial release
- [ ] **New-user test:** someone who hasn't used Locus follows the in-app "Indoor crime scene" guide on the sample case (Guides → Create the sample case…), timed; the target is under 30 minutes. Note where they got stuck.
- [x] **Installer:** WiX approved (2026-09-24). - [ ] Code-signing certificate, only if it's ever sold.
- [x] **Updates:** GitHub Releases (2026-09-24). I'll make the update-signing key.
- [x] **No licence:** not needed; every tool works (2026-09-24).
- [ ] **Licence private key:** move `E:\Claude\scratch\locus\license\locus-license-private-key.txt` somewhere safe and backed up (not the repository). Anyone with it can make licences.
- [ ] **Legal review** of THIRD_PARTY_NOTICES.txt.
