# Validation report

`locus-validate run --full --out validation.pdf` runs every tool end to end against synthetic ground truth, many times, and prints a PDF in the analysis reports' layout. It is regenerated for every release: the release job in CI runs it and keeps the PDF with the release. `locus-validate run` without `--full` is a quick check with a few cases per tool. The run exits non-zero if any bound isn't met.

For each tool the report gives:
- the error of each case against the truth, with its mean, standard deviation, 95th percentile and worst;
- how often the tool's stated 95 % interval or region contained the truth (coverage);
- the bound it is held to, and whether it was met.

Coverage well below 95 % (under 90 % over 20 or more cases) is flagged in "Needs attention": it means the stated uncertainty is too small.

| Tool | Cases (full) | Ground truth | Bound (SPEC Phase 13) |
|---|---|---|---|
| Bullet trajectory | 400 | Synthetic rooms of perforated panels; defects picked with picking and scan noise | every run under 0.5° |
| Bloodstain area of origin, from photos | 8 rooms, about 200 stains each | Stains rendered as photos and measured as the app does (fiducial alignment, automatic edges, ellipse fit) | mean under 10 cm |
| Bloodstain area of origin, hand-measured | 100 | The same rooms with hand measurement's noise: a poor case, reported for its coverage | none (coverage reported) |
| Bloodstain conventional point | as above | The same stains | none (shown for comparison) |
| Height, good camera solves | 40 seeds × 2 marker counts × 2 cameras | Rendered CCTV and handheld views of people of known height | 95 % of errors under 2 cm |
| Height, all solves | as above | as above | none (coverage reported) |
| Scan registration | 5 | Four-station synthetic rooms, 1M points per scan, rough poses off by 0.5 m and 5° | every run under 2 mm |
| Volumetric crush | 40 | Synthetic vehicles with a dent of known volume, registered to an exemplar | none (the interval is deliberately conservative) |
| Crash formulas | 3 | Closed-form skid speeds | under 0.01 % |
| Animation motion | 121 frames | A constant speed on a curve | under 0.001 mm |
| Photogrammetry | recorded | ETH3D with laser-scan truth, recorded 2026-09-23 and **not rerun** (COLMAP and the benchmark stay outside the repository) | 99 % of distances within 1 % |

**Crash cases:** crush energy is checked against the NHTSA CRASH3 manual's sample run in the unit tests. The published textbook cases for skid, yaw and momentum are awaited (docs/ADDISON-TODO.md).

**Reproducibility:** the generators, their seeds and the suite are in the repository (`crates/locus-synth`, `crates/locus-validate/src/suite.rs`), so every number can be regenerated. The per-tool acceptance tests in `crates/*/tests` guard the same bounds on every push.

**Limits:** synthetic truth is not casework. The physical studies that complete validation are described in [validation-protocol.md](validation-protocol.md).
