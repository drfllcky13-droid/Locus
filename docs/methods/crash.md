# Crash reconstruction: skid, yaw, momentum and crush energy

Four standard calculations, each done with the uncertainty of its inputs. Each run is stored as an audit-logged analysis record with a PDF report.

- Code: `crates/locus-analysis/src/crash.rs` (pure, tested against hand-worked examples).
- Commands: `src-tauri/src/crash_cmds.rs`.
- Reports: `crates/locus-report/src/crash.rs`.
- UI: `app/src/tools/crash/CrashPanel.tsx`.
- Vehicle specifications: `app/src/scene3d/library.ts` (`vehicleSpec`).

SI units throughout (m, s, kg, N, J), and g = 9.80665 m/s² (standard gravity). Speeds are reported in m/s with km/h beside them.

## Inputs, ranges and results

Every input is a value with a range, entered as a value ± tolerance. Each result is given three ways:

1. **From the inputs' values.**
2. **By the range method:** the smallest and largest result over every corner of the inputs' ranges (all 2ᵏ combinations of each ranged input at its low or high end; above 16 ranged inputs, the Monte Carlo's extremes stand in). This is the traditional reconstruction bound, and for results monotonic in each input, which the skid, yaw and crush formulas are, it is exact.
3. **By Monte Carlo:** 20,000 draws, each input uniform over its range (or normal with the range as ±2σ, for measured values with 95 % bounds), seeded so a run repeats exactly. It gives the mean, 1σ and a 95 % interval. Draws with no real answer, such as a negative v², are counted and left out.

## Vehicle specifications

A vehicle model is built from:
- its overall length, width and height;
- wheelbase;
- front overhang (the rear overhang is length − wheelbase − front overhang);
- front and rear track;
- tyre diameter;
- optionally, mass and centre-of-gravity height.

The wheels sit where these put them. Missing values are derived from the vehicle class exactly as models were built before (axles centred, tracks 0.3 m inside the width, tyres 0.44 × height up to 0.9 m), so scenes stored earlier don't change. The yaw tool's offset from the mark to the centre of mass's path is half the track.

## Speed from skid marks

For each stretch of mark on one surface, the effective drag factor is f = μ n cos θ + sin θ:
- μ is the surface's drag factor;
- n is the braking efficiency, the share of the full drag the braked wheels give;
- θ = atan(grade), with grade as rise over run, positive uphill.

The speed at the start of the marks is v = √(v_end² + Σ 2 g fᵢ dᵢ), for mark lengths dᵢ and a speed v_end at the end (0 if the vehicle stopped). This equals combining the stretches' speeds as √(v₁² + v₂² + …).

A stretch can be measured on the cloud as a polyline of picked points. Each point is resolved again from stored data, and the stored length must match it.

Worked in the tests (g = 9.80665):

| Case | Working | Result |
|---|---|---|
| 30 m, μ 0.7, level | √(2 g · 0.7 · 30) = √411.879 | 20.2948 m/s |
| 30 m, μ 0.7, 5 % uphill | f = 0.7 cos 2.862° + sin 2.862° = 0.749064 | 20.9940 m/s |
| 30 m, μ 0.7, n 0.8 | √(2 g · 0.56 · 30) | 18.1522 m/s |
| 20 m at 0.7, then 10 m at 0.4 | √(2 g (14 + 4)) | 18.7893 m/s |
| μ 0.6–0.8 over 30 m | range method | 18.7893 to 21.6961 m/s |

## Critical speed from a yaw mark

**Radius from a chord.** R = C²/(8M) + M/2, from a chord C across the mark and the middle ordinate M from the chord's midpoint to the mark.

**Radius from points.** Alternatively, a circle is fitted to points picked along the mark on the cloud:
- a plane is fitted to the points;
- in that plane the algebraic (Kåsa) circle gives the start;
- Gauss–Newton then minimises the points' distances from the circle.

The radius's 1σ comes from the fit's covariance and residuals, and never falls below what the scan points' σ allows. It enters the Monte Carlo as a normal input. The report shows:
- the arc the points span, warned about under 20°;
- the RMS distance of the points from the circle, warned about over 50 mm (a point on a kerb or a wall pulls the circle).

**Centre-of-mass offset.** The radius is reduced by the offset from the mark to the centre of mass's path: half the track, for the outside front tyre's mark.

**Critical speed.** v = √(g R (μ + e)/(1 − μ e)) for superelevation e (the cross slope toward the centre), which is √(μ g R) on the level.

Worked in the tests:
- R = 30²/(8 · 1.5) + 0.75 = 75.75 m, and v = √(0.7 g · 75.75) = 22.8035 m/s.
- With e = 0.05, v = √(g · 75.75 · 0.75/0.965) = 24.0281 m/s.
- A fitted circle of radius 3 m over 100° of arc is recovered exactly.
- Twelve points over 40° of a 60 m circle, on a 2 % slope with 5 mm noise, give the radius to within 0.2 m.

## Linear momentum, two vehicles

Conservation of linear momentum in the plane:

m₁ v₁ d(θ₁) + m₂ v₂ d(θ₂) = m₁ u₁ d(φ₁) + m₂ u₂ d(φ₂)

- m: the masses;
- θ: the approach directions;
- φ: the departure directions;
- u: the departure speeds;
- d(·): the unit vector of a direction, clockwise from project north.

It is solved for the impact speeds v₁ and v₂ (a 2 × 2 linear system). Each vehicle's delta-V is |v d(θ) − u d(φ)|.

The system is singular when the approach directions are parallel. A separation outside 20°–160° is warned about, because small input errors then give large speed errors.

**Sensitivity table.** Each ranged input is moved to its low and then its high end with the others at their values, and both impact speeds are reported. This is the one-at-a-time sensitivity table of reconstruction practice.

Worked in the tests:
- A 1,500 kg car northbound at 20 m/s and a 1,200 kg car eastbound at 15 m/s lock together. The momentum is (18,000, 30,000) kg·m/s; they depart at 30.964° from north at 34,985.7/2,700 = 12.9577 m/s. Momentum gives back 20 and 15 m/s exactly.
- Sensitivity: with B's mass at 1,100 kg and the departure held, P = 2,600 · 12.9577, so v_A = P cos φ/1,500 = 19.2593 m/s and v_B = P sin φ/1,100 = 15.7576 m/s. The table reproduces both.

## Crush energy (Campbell / CRASH3)

**The model.** The force per unit width is linear in residual crush c, A + B c (Campbell 1974; the CRASH3 model). Each strip of the damage therefore absorbs A c + B c²/2 + G per unit width, with G = A²/(2B).

**The profile.** The depths C₁ … Cₙ are equally spaced across the damage width L (2, 4 or 6 in the usual protocols) and vary linearly between measurements. Each span of width Δ = L/(n − 1) integrates exactly to:

Δ [A (c₁ + c₂)/2 + B (c₁² + c₁c₂ + c₂²)/6 + G]

**Force direction.** The sum is multiplied by (1 + tan² α) for a principal direction of force α off the face's normal (|α| ≤ 45°).

**Equivalent barrier speed.** √(2E/m).

**Stiffness coefficients.** A and B are entered with their source, which the report prints. A bundled coefficient table from public NHTSA crash-test data is planned (it needs the data downloaded, with permission, and its licence recorded; rule 7).

Worked in the tests, with A = 50,000 N/m and B = 1,000,000 N/m² (G = 1,250 N):

| Case | Working | Result |
|---|---|---|
| Uniform 0.3 m over 1.5 m (2 or 6 points) | 1.5 (15,000 + 45,000 + 1,250) | 91,875 J |
| Triangle 0 to 0.3 m | 1.5 (7,500 + 15,000 + 1,250) | 35,625 J |
| Force 30° off the normal | × (1 + tan² 30°) = × 4/3 | |
| 1,500 kg | √(2 · 91,875/1,500) | 11.0680 m/s |

## Validation

Every formula is tested against the examples worked by hand above, including the range method's extremes and the sensitivity table. The examples were worked independently here; published textbook examples are cited, not copied.

In the app, on a generated room's scan:
- a skid stretch was measured on the cloud and combined with a second, entered surface;
- a yaw mark's radius was fitted to picked points;
- momentum and crush were entered.

Each run was saved and printed. Momentum and crush reproduced the worked values.

## Assumptions and limitations

Each tool stores its own assumptions and limitations with every run and prints them in its report (see the code's `*_ASSUMPTIONS` and `*_LIMITATIONS`). The principal limitations:

- **Skid.** The time to lock the wheels isn't counted, so the result is a lower bound on the speed when braking began. Anti-lock brakes may leave faint or no marks. The drag factor is the most uncertain input.
- **Yaw.** Braking or accelerating during the yaw, a tyre that isn't the outside front, or a radius measured late in the mark all bias the result.
- **Momentum.** Nearly parallel approaches can't separate the speeds, and the departure speeds usually dominate the uncertainty.
- **Crush.** The stiffness coefficients are extrapolated from tests. The equivalent barrier speed is neither the impact speed nor a delta-V.

## References

- J. C. Collins, *Accident Reconstruction*, Charles C. Thomas, 1979.
- L. B. Fricke, *Traffic Accident Reconstruction*, Northwestern University Traffic Institute, 1990.
- R. M. Brach and R. M. Brach, *Vehicle Accident Analysis and Reconstruction Methods*, 2nd ed., SAE International, 2011.
- K. L. Campbell, "Energy basis for collision severity", SAE technical paper 740565, 1974.
- National Highway Traffic Safety Administration, *CRASH3 Technical Manual*, US Department of Transportation, 1986.
- J. A. Neptune, "Crush stiffness coefficients, restitution constants, and a revision of CRASH3 and SMAC", SAE technical paper 980024, 1998.
