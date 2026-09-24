// The diagram symbol library (assets/symbols: original, drawn for Lotus), the legend built
// from what a diagram uses, and evidence-marker numbering.
import index from "../../../assets/symbols/index.json";
import type { Diagram, Entity } from "./model";

const raw = import.meta.glob("../../../assets/symbols/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

export interface SymbolDef {
  id: string;
  name: string;
  /** Default size on the ground (m): the symbol's 100-unit box spans this. */
  size: number;
  /** SVG content inside the 100 × 100 box centred on the origin, in currentColor. */
  body: string;
}

/** The markup inside a symbol file's <svg> element, without its <title>. */
function inner(svg: string): string {
  return svg
    .replace(/^[\s\S]*?<svg[^>]*>/, "")
    .replace(/<\/svg>\s*$/, "")
    .replace(/<title>[\s\S]*?<\/title>/, "");
}

export const SYMBOLS: SymbolDef[] = (index as { id: string; name: string; size: number }[]).map(
  (s) => ({ ...s, body: inner(raw[`../../../assets/symbols/${s.id}.svg`] ?? "") }),
);

export const symbolById = new Map(SYMBOLS.map((s) => [s.id, s]));

export interface LegendItem {
  key: string;
  label: string;
  /** Symbol id to draw beside the label, or a built-in glyph. */
  glyph: { symbol: string } | "marker" | "point" | "measured";
  count: number;
}

type Marker = Extract<Entity, { kind: "marker" }>;

/** "1–3, 5, 7–9" */
function ranges(ns: number[]): string {
  const s = [...new Set(ns)].sort((a, b) => a - b);
  const out: string[] = [];
  for (let i = 0; i < s.length;) {
    let j = i;
    while (j + 1 < s.length && s[j + 1] === s[j] + 1) j++;
    out.push(i === j ? `${s[i]}` : `${s[i]}–${s[j]}`);
    i = j + 1;
  }
  return out.join(", ");
}

/**
 * What the legend lists: every symbol in use (in library order, with counts), evidence
 * markers with their numbers, and reference and measured points. Derived from the document
 * each time, so it is always current.
 */
export function legendItems(doc: Diagram): LegendItem[] {
  const hidden = new Set(doc.layers.filter((l) => !l.visible).map((l) => l.id));
  const shown = doc.entities.filter((e) => !hidden.has(e.layer));
  const items: LegendItem[] = [];
  for (const s of SYMBOLS) {
    const n = shown.filter((e) => e.kind === "symbol" && e.symbol === s.id).length;
    if (n) items.push({ key: `symbol:${s.id}`, label: s.name, glyph: { symbol: s.id }, count: n });
  }
  const markers = shown.filter((e): e is Marker => e.kind === "marker");
  if (markers.length)
    items.push({
      key: "markers",
      label: `Evidence markers ${ranges(markers.map((m) => m.number))}`,
      glyph: "marker",
      count: markers.length,
    });
  const points = shown.filter((e) => e.kind === "point");
  const measured = points.filter((e) => e.kind === "point" && e.measurement).length;
  if (points.length - measured)
    items.push({
      key: "points",
      label: "Reference point",
      glyph: "point",
      count: points.length - measured,
    });
  if (measured)
    items.push({
      key: "measured",
      label: "Measured point (tape)",
      glyph: "measured",
      count: measured,
    });
  return items;
}

/** The number for a new evidence marker: one more than the highest so far. */
export function nextMarker(doc: Diagram): number {
  return doc.entities.reduce((n, e) => (e.kind === "marker" ? Math.max(n, e.number) : n), 0) + 1;
}

/**
 * Renumber evidence markers 1…n in reading order across the diagram: top to bottom, then
 * left to right (rows within 0.5 m count as one row).
 */
export function renumberMarkers(doc: Diagram): Diagram {
  const markers = doc.entities.filter((e): e is Marker => e.kind === "marker");
  const order = [...markers].sort((a, b) =>
    Math.abs(a.at[1] - b.at[1]) > 0.5 ? b.at[1] - a.at[1] : a.at[0] - b.at[0],
  );
  const number = new Map(order.map((m, i) => [m.id, i + 1]));
  return {
    ...doc,
    entities: doc.entities.map((e) =>
      e.kind === "marker" ? { ...e, number: number.get(e.id)! } : e,
    ),
  };
}
