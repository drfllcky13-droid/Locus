// How measurements read in the UI. Values arrive in SI units from Rust (rule 3) and are
// only converted here, at the display boundary. Every value shows its unit and its 1σ.

export interface Measured {
  value: number;
  sigma: number;
}

export interface MeasurementRecord {
  id: number;
  kind: "distance" | "angle" | "area" | "height";
  points: { scan: string; index: number; project: [number, number, number] }[];
  result: Record<string, unknown> & { sigma_point_m: number };
  created_at: string;
  created_by: string;
}

/** Millimetre resolution: three decimals of a metre, so a displayed coordinate is within 0.5 mm. */
export function formatCoordinate(p: [number, number, number]): string {
  return `x ${p[0].toFixed(3)} m, y ${p[1].toFixed(3)} m, z ${p[2].toFixed(3)} m`;
}

function sigmaLength(s: number): string {
  return s < 0.1 ? `${(s * 1000).toFixed(1)} mm` : `${s.toFixed(3)} m`;
}

export function formatLength(m: Measured): string {
  return `${m.value.toFixed(3)} m ± ${sigmaLength(m.sigma)}`;
}

export function formatMeasurement(rec: MeasurementRecord): { label: string; detail: string } {
  const r = rec.result;
  // On a photogrammetric cloud the run's model sets the uncertainty: max(percent × length, floor).
  const ph = r.photogrammetry as
    { analysis: number; percent: number; floor_m: number; from_checks: boolean } | undefined;
  const sp = ph
    ? `photogrammetric cloud (analysis ${ph.analysis}): 1σ at least ${(ph.percent * 100).toFixed(2)} % of the length and ${(ph.floor_m * 1000).toFixed(1)} mm, from ${ph.from_checks ? "the case's checks" : "the benchmark"}`
    : `assumes ${(r.sigma_point_m * 1000).toFixed(1)} mm per point, 1σ`;
  switch (rec.kind) {
    case "distance": {
      const m = r as unknown as Measured;
      return { label: formatLength(m), detail: sp };
    }
    case "angle": {
      const m = r as unknown as Measured;
      const deg = (v: number) => (v * 180) / Math.PI;
      return { label: `${deg(m.value).toFixed(2)}° ± ${deg(m.sigma).toFixed(2)}°`, detail: sp };
    }
    case "area": {
      const a = r as unknown as {
        area: Measured;
        perimeter: number;
        plane: { rms: number; max_abs: number };
      };
      return {
        label: `${a.area.value.toFixed(3)} m² ± ${a.area.sigma.toFixed(4)} m²`,
        detail: `perimeter ${a.perimeter.toFixed(3)} m; outline off its plane by ${sigmaLength(a.plane.rms)} RMS, ${sigmaLength(a.plane.max_abs)} max; ${sp}`,
      };
    }
    case "height": {
      const h = r as unknown as { height: Measured; plane: { rms: number; max_abs: number } };
      return {
        label: formatLength(h.height),
        detail: `reference plane fits its points to ${sigmaLength(h.plane.rms)} RMS; ${sp}`,
      };
    }
  }
}

/** Points each tool needs before it can finish; `min` for open-ended tools. */
export const TOOL_POINTS: Record<
  MeasurementRecord["kind"],
  { exact?: number; min?: number; hint: string }
> = {
  distance: { exact: 2, hint: "Click two points." },
  angle: {
    exact: 3,
    hint: "Click a point on the first arm, the vertex, then a point on the second arm.",
  },
  area: { min: 3, hint: "Click the outline's corners in order, then press Enter." },
  height: {
    min: 4,
    hint: "Click three or more points on the reference surface, press Enter, then click the point to measure.",
  },
};
