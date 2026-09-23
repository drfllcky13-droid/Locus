# Hand measurements

Positions of points measured at the scene with a tape, relative to points whose positions are known (reference points). Code: `crates/locus-analysis/src/handmeasure.rs`; entry in the diagram editor's **Measured point** tool (`app/src/diagram2d/MeasureDialog.tsx`).

Positions are plan-view (2-D) project metres. The taped readings are stored with the point they produced (which reference points, which distances, which side), so any point in a diagram can be traced back to its field notes.

## Methods

**Baseline and offset.** A baseline runs from reference point A to reference point B. The point is `along` metres from A toward B, then `offset` metres square to the baseline, on the stated side (left or right of the direction A→B):

    P = A + along · u + offset · n,   u = (B − A)/|B − A|,   n = u turned 90° to the chosen side.

**Triangulation (trilateration).** The point is at the taped distance from each of two or more reference points.

- *Two references:* the two circles meet in two points, one on each side of the line from the first reference to the second, and the examiner states which. The solution is exact (closed form):

      x = (r₁² − r₂² + d²) / 2d,   h = √(r₁² − x²),   P = A + x · u ± h · n.

  Tapes that miss each other by less than 0.1 mm are treated as touching (one solution, on the baseline). If they miss by more, the distances can't all be right, and the tool says by how much.
- *Three or more references:* the position minimising Σ (|P − Qᵢ| − dᵢ)² (Gauss–Newton). Both starting sides are tried and the better fit kept, so no side needs to be stated unless the references are all in a line. The residual of each tape (distance to the solution minus the reading) is reported. If any residual exceeds three times that tape's stated precision, the tool warns that the tapes disagree beyond their precision.

## Uncertainty

Each reading is uncertain by `fixed + per_metre × distance` (1σ; the examiner sets both, default 2 mm + 1 mm per metre), and each reference point by a stated 1σ per axis. The covariance of the position is propagated to first order: a numerical Jacobian of the solution with respect to every input (each reference coordinate and each distance), assuming independent inputs. The tool shows the 1σ radius (square root of the covariance's trace), and the point keeps it.

Tests (`handmeasure.rs`): exact reproduction of known positions from exact distances, to 10⁻¹² m, on both sides of the baseline, near and far, and with three references; hand-checked baseline/offset values; the propagated covariance agrees with a 20,000-run Monte Carlo within 5 %; a third tape misread by 10 cm is flagged; impossible and ambiguous inputs are refused.

## Limitations

- Plan view only: slopes are not corrected. A tape laid along a slope of angle θ reads 1/cos θ of the horizontal distance (0.4 % at 5°), so steep ground needs slope-corrected readings.
- The uncertainty is first order. With two references, it grows without bound as the circles become tangent (the point lies close to the line through the references), and is then an underestimate. Choose references so the circles cross at a good angle.
- Errors in reference points' positions are assumed independent of each other.
- A point measured from points that were themselves measured inherits their positions, but not their correlation: its stated uncertainty uses the reference precision the examiner enters.
