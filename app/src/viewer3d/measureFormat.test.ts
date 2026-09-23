import { expect, test } from "vitest";
import {
  formatCoordinate,
  formatLength,
  formatMeasurement,
  type MeasurementRecord,
} from "./measureFormat";

test("a coordinate far from the origin is shown within 1 mm of the source (item 6)", () => {
  // Resolved in Rust from the stored f64 point; the UI must not lose the millimetres.
  const source: [number, number, number] = [500123.4567, 4400456.789, 312.3456];
  const shown = formatCoordinate(source);
  expect(shown).toBe("x 500123.457 m, y 4400456.789 m, z 312.346 m");
  const back = [...shown.matchAll(/-?\d+\.\d+/g)].map((m) => Number(m[0]));
  back.forEach((v, i) => expect(Math.abs(v - source[i])).toBeLessThanOrEqual(0.0005));
});

const rec = (kind: MeasurementRecord["kind"], result: object): MeasurementRecord => ({
  id: 1,
  kind,
  points: [],
  result: { sigma_point_m: 0.002, ...result },
  created_at: "",
  created_by: "",
});

test("every value shows its unit and 1σ", () => {
  expect(formatLength({ value: 13, sigma: 0.002828 })).toBe("13.000 m ± 2.8 mm");
  expect(formatMeasurement(rec("angle", { value: Math.PI / 2, sigma: 0.004 })).label).toBe(
    "90.00° ± 0.23°",
  );
  const area = formatMeasurement(
    rec("area", {
      area: { value: 1, sigma: 0.0021 },
      perimeter: 4,
      plane: { rms: 0.0004, max_abs: 0.0009 },
    }),
  );
  expect(area.label).toBe("1.000 m² ± 0.0021 m²");
  expect(area.detail).toContain("0.4 mm RMS");
  expect(formatMeasurement(rec("distance", { value: 1, sigma: 0.001 })).detail).toBe(
    "assumes 2.0 mm per point, 1σ",
  );
  expect(
    formatMeasurement(
      rec("distance", {
        value: 3,
        sigma: 0.0105,
        photogrammetry: { analysis: 4, percent: 0.0035, floor_m: 0.006, from_checks: false },
      }),
    ).detail,
  ).toBe(
    "photogrammetric cloud (analysis 4): 1σ at least 0.35 % of the length and 6.0 mm, from the benchmark",
  );
});
