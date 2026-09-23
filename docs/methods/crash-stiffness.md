# CRASH3 stiffness coefficients from NHTSA crash tests

The crush-energy tool needs a vehicle's frontal stiffness coefficients A (N/m) and B (N/m²). Locus bundles a table derived from public NHTSA barrier tests. An examiner can use it, or override it with values entered with their source.

- Code: `crates/locus-analysis/src/stiffness.rs` (derivation, tested).
- Data: `crates/locus-analysis/data/nhtsa-frontal-barrier.csv`.
- Fetcher: `tools/nhtsa/fetch.py`.

## Data source

The data comes from the NHTSA Vehicle Crash Test Database, through its public API at https://nrd.api.nhtsa.dot.gov/, retrieved on 2026-09-23. It is a work of the US Government, not subject to copyright in the United States (17 U.S.C. §105), and data.gov lists it under the USA.gov Public Domain License 1.0. It is recorded in THIRD_PARTY_NOTICES.txt as `LicenseRef-US-Government-Works`.

`tools/nhtsa/fetch.py` lists every test, by test-number range because the API's paging repeats and skips rows (10,683 when fetched), and keeps those that meet all of these:
- configuration "VEHICLE INTO BARRIER";
- impact angle 0°;
- no offset;
- a rigid flat barrier.

That leaves 3,060 tests. For each test vehicle it writes one row:
- the test number and date;
- the test type;
- make, model and model year;
- body type;
- test mass;
- impact speed;
- vehicle width and recorded damage width;
- the principal direction of force;
- C1–C6;
- the damage index.

Values are kept as published, one row for each of the 1,056 test vehicles. Nothing is derived in the fetcher.

When parsed, 850 rows have a usable mass, speed, width and crush profile. They cover 698 vehicles (make, model, model year), and 612 of those have a single test.

**Reading the profile.** Older tests record two points (C1, C2) or four (C1–C4) and store zeros for the rest. So a row whose C3–C6 are all zero is read as two points, and one whose C5 and C6 are zero as four. Otherwise all six are used. Small negative depths (to −10 mm, measurement noise) read as 0. A row with a larger negative depth is left out.

**Damage width.** The damage width is the recorded indentation length. When that is 0 or missing, as in many full-width frontal tests, the vehicle's overall width is used and the entry is flagged.

## Derivation: Campbell's method with the CRASH3 energy integral

The model is Campbell's (1974), as used in CRASH3. The force per unit width is linear in residual crush c, with A = m b₀ b₁ / L and B = m b₁² / L. Here:
- m is the test mass;
- L is the damage width;
- b₀ is the impact speed below which no permanent crush occurs;
- b₁ is the slope of impact speed against crush.

This gives G = A²/2B = m b₀² / 2L.

For each test, b₁ is chosen so that the crush energy over the measured profile, by the same integral the crush tool uses (the CRASH3 manual's equations (2)–(4)), equals the kinetic energy ½ m V². Per unit mass that is:

Q b₁² + 2 b₀ C̄ b₁ + (b₀² − V²) = 0

where C̄ is the profile's mean crush and Q the mean of its square, linear between the points. The positive root is taken.
- For uniform crush, Q = C̄² and this is Campbell's b₁ = (V − b₀)/C̄.
- For other profiles it makes each test's own coefficients reproduce its own kinetic energy exactly. The tests check this for every bundled test.

**b₀** is taken as 8 km/h (5 mph), with a range of ±3.2 km/h (2 mph), following common practice for frontal barrier data.

## Uncertainty

**Each test.** A 400-draw Monte Carlo draws:
- b₀ uniformly over 8 ± 3.2 km/h;
- the impact speed with 0.5 km/h 1σ;
- each crush depth with 10 mm 1σ;
- the width with 1 % 1σ.

The standard deviation of A and B over the draws is the test's own 1σ. It is seeded by the test number, so it repeats exactly.

**Each vehicle** (tests grouped by make, model and model year):
- the value is the mean over its tests;
- 1σ is √(mean of the tests' σ² + the sample variance between tests), so test-to-test variation adds to the measurement uncertainty.

**Single-test vehicles** are flagged. In the app, the report's "Needs attention" says their uncertainty can't include test-to-test variation.

In the crush tool, a table entry's A and B enter as normal inputs, with ±2σ as the range.

## Examiner override

Instead of the table, the examiner can enter A and B, each with a tolerance, and a source (required). The report prints the source, or, for a table entry, the vehicle, its NHTSA test numbers and each test's coefficients.

The table is for frontal damage. When the damaged face isn't the front, the report says so.

## Checks

- **Hand-worked (uniform crush).** 1,500 kg at 56 km/h, 0.5 m of crush over 1.5 m, b₀ = 8 km/h: b₁ = 26.667 /s, A = 59,259.3 N/m and B = 711,111 N/m².
- **Energy balance.** Every bundled test's coefficients reproduce its kinetic energy over its own profile (to 10⁻⁶).
- **Plausibility.** The table's median frontal A is 64 kN/m (5–95 % 38–134 kN/m) and median B 769 kN/m² (365–2,561 kN/m²). The CRASH3 manual's category 4 front values (Table 8-2: 356 lb/in = 62 kN/m, 34 lb/in² = 527 kN/m²) lie in that range.
- **The CRASH3 manual's sample run** (§5, damage summary on its page 73), reproduced by the crush-energy integral:

  | Vehicle | Inputs | Printed energy | Computed | Range from the inputs' rounding (±0.05 in) |
  |---|---|---|---|---|
  | 1: category 4 front (A 356 lb/in, B 34 lb/in², G 1,874 lb) | L 73.0 in, C1 2.7, C2 3.6 in | 19,245.3 ft-lb | 19,255.1 | 19,101.5 to 19,409.6 |
  | 2: category 4 side (A 143, B 50, G 203) | L 84.5 in, C 6.2, 8.3, 9.2, 5.9, 4.4, 0.8 in; 45° | 31,220.8 ft-lb | 31,099.9 | 30,761.5 to 31,440.5 |

  The printout gives its inputs to 0.1 in; both printed energies lie within the range the formula gives over that rounding. The manual lists G beside A and B (not rounded as A²/2B), so the check uses its G.

  Vehicle 1's printed energy has no oblique correction, though its ANG is −45°. The manual's flow chart applies the (1 + tan² α) correction only on one branch of its damage subroutine, and the text limits it to ±75°. Vehicle 2's energy includes it.

## Limitations

- The coefficients come from full-width frontal rigid-barrier tests. They describe the front only, at the test speeds (mostly 48–56 km/h), and are extrapolated to other impacts.
- b₀ is assumed, not measured. Its range dominates A's uncertainty.
- Grouping by make, model and model year doesn't merge a model's generations, or tell body variants apart beyond the model name. Test mass includes the test's ballast and instruments.
- When a test's damage width wasn't recorded, the vehicle's width stands in, and the entry is flagged.

## References

- K. L. Campbell, "Energy basis for collision severity", SAE technical paper 740565, 1974.
- National Highway Traffic Safety Administration, *CRASH3 User's Guide and Technical Manual*, US Department of Transportation (NTIS PB83-112201): the crush-energy integral (§9, equations (2)–(4)), the oblique correction, Table 8-2 and the §5 sample run.
- J. A. Neptune, "Crush stiffness coefficients, restitution constants, and a revision of CRASH3 and SMAC", SAE technical paper 980024, 1998.
- National Highway Traffic Safety Administration, Vehicle Crash Test Database, https://nrd.api.nhtsa.dot.gov/.
