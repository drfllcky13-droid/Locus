// Display formatting. Every number shown carries its unit (CLAUDE.md rule 3).
import type { Contents, EvidenceStatus, IntegrityReport, LinearUnit } from "./api";

/** `metres`: one unit in metres, as locus-core's LinearUnit::meters. */
export const UNITS: { value: LinearUnit; label: string; symbol: string; metres: number }[] = [
  { value: "meter", label: "Meters", symbol: "m", metres: 1 },
  { value: "centimeter", label: "Centimeters", symbol: "cm", metres: 0.01 },
  { value: "millimeter", label: "Millimeters", symbol: "mm", metres: 0.001 },
  { value: "foot", label: "International feet", symbol: "ft", metres: 0.3048 },
  { value: "us_survey_foot", label: "US survey feet", symbol: "US ft", metres: 1200 / 3937 },
  { value: "inch", label: "Inches", symbol: "in", metres: 0.0254 },
];

export function unitSymbol(u: LinearUnit | null): string {
  return UNITS.find((x) => x.value === u)?.symbol ?? "units";
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let v = n;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < units.length - 1);
  return `${v.toFixed(v < 10 ? 2 : 1)} ${units[i]} (${n.toLocaleString("en-US")} bytes)`;
}

export function formatCount(n: number, noun: string, plural = `${noun}s`): string {
  return `${n.toLocaleString("en-US")} ${n === 1 ? noun : plural}`;
}

/** Extent of bounds along each axis, in the source unit. */
export function formatExtent(
  b: { min: number[]; max: number[] } | null,
  unit: LinearUnit | null,
): string {
  if (!b) return "no valid points";
  const s = unitSymbol(unit);
  return b.min.map((lo, i) => `${(b.max[i] - lo).toFixed(3)} ${s}`).join(" × ");
}

/** Mirrors `Contents::needs_unit` in locus-core. */
export function needsUnit(c: Contents): boolean {
  return c.declared_unit === null && (c.scans.length > 0 || c.meshes.length > 0);
}

/** One line per integrity problem, or an empty list when every file matches. */
export function integrityProblems(r: IntegrityReport): string[] {
  const describe = (s: EvidenceStatus) =>
    s.status === "missing"
      ? "is missing from the evidence folder"
      : s.status === "changed"
        ? `has changed: its SHA-256 is now ${s.actual}`
        : "";
  return [
    ...r.results
      .filter(([, s]) => s.status !== "intact")
      .map(([id, s]) => `Evidence #${id} ${describe(s)}`),
    ...r.unrecorded.map((f) => `${f} is in the evidence folder but was never imported`),
  ];
}

export function progressPercent(done: number, total: number): number {
  return total > 0 ? Math.min(100, Math.floor((done / total) * 100)) : 0;
}
