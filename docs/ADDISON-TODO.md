# Addison's to-do list

Things only you can do. Nothing here blocks the next phases, but each is needed before release unless noted. Tick them off as you go.

## Checks with your hands or hardware
- [ ] **Case package on a clean PC.** Make a case package (3D view → Export → Make a case package…), copy it to a USB drive, and open `Locus Viewer.exe` on a Windows PC that has never had Locus, logged in as a standard user (no admin). Check that it opens, says all files match, and shows the package hash under Help → About.
- [ ] **1:100 print check.** Print a diagram PDF at actual size (no "fit to page"), then measure the 10 m line (should be 100 mm) and the 100 mm calibration bar with a ruler.
- [ ] **Mid-range GPU.** Run Locus on a PC with a mid-range graphics card (for example an RTX 3060 or GTX 1660), open a large scan, and tell me roughly how smooth navigation is (or run Help → About's benchmark if I add one).

## Decisions and information
- [ ] **Textbook worked examples** for skid, yaw and momentum. Which book (Fricke / Northwestern, Daily et al. / IPTM, or Brach & Brach / SAE), plus the inputs and published answers for 3–5 examples each.
- [ ] **Publisher domain** for the app's identifier (needed before the first signed release).
- [ ] **Your lab's trajectory angle convention**, and whether the lab uses plan-view convergence of floor bloodstains.
- [ ] **Final product name** ("Locus" is a working name), and whether it will be sold.
- [ ] **Physical validation studies:** who will run them, and when (the protocol will be in `docs/methods/validation-protocol.md`).
- [ ] **Evidence.com:** whether to pursue integration through Axon's partner program.
- [ ] **GitHub Actions minutes:** whether to stay on the free plan (CI is trimmed to fit it).

## Before any commercial release
- [ ] **Legal review** of THIRD_PARTY_NOTICES.txt.
