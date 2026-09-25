// Plane geometry and snapping for the diagram editor. Pure functions; project metres.
import type { Entity, Pt } from "./model";

export const sub = (a: Pt, b: Pt): Pt => [a[0] - b[0], a[1] - b[1]];
export const add = (a: Pt, b: Pt): Pt => [a[0] + b[0], a[1] + b[1]];
export const scale = (a: Pt, k: number): Pt => [a[0] * k, a[1] * k];
export const dist = (a: Pt, b: Pt): number => Math.hypot(a[0] - b[0], a[1] - b[1]);
export const mid = (a: Pt, b: Pt): Pt => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];

/** Foot of the perpendicular from `p` to the infinite line through `a` and `b`. */
export function foot(p: Pt, a: Pt, b: Pt): Pt | null {
  const d = sub(b, a);
  const len2 = d[0] * d[0] + d[1] * d[1];
  if (len2 < 1e-24) return null;
  const t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2;
  return add(a, scale(d, t));
}

/** Point on an arc at angle `t`. */
export const onArc = (c: Pt, r: number, t: number): Pt => [
  c[0] + r * Math.cos(t),
  c[1] + r * Math.sin(t),
];

/** The straight segments of an entity, for midpoints and perpendiculars. */
export function segments(e: Entity): [Pt, Pt][] {
  switch (e.kind) {
    case "line":
    case "dimension":
      return [[e.a, e.b]];
    case "polyline": {
      const s: [Pt, Pt][] = [];
      for (let i = 1; i < e.points.length; i++) s.push([e.points[i - 1], e.points[i]]);
      if (e.closed && e.points.length > 2) s.push([e.points[e.points.length - 1], e.points[0]]);
      return s;
    }
    case "room":
    case "road":
      return e.geometry.segments.map((s) => [s.a, s.b]);
    default:
      return [];
  }
}

/** Points worth snapping to exactly: ends, vertices, arc ends and centres, placed points. */
export function endpoints(e: Entity): Pt[] {
  switch (e.kind) {
    case "line":
    case "dimension":
      return [e.a, e.b];
    case "polyline":
      return e.points;
    case "room":
    case "road":
      return e.geometry.segments.flatMap((s) => [s.a, s.b]);
    case "arc":
      return [onArc(e.center, e.radius, e.start), onArc(e.center, e.radius, e.end), e.center];
    case "point":
    case "marker":
    case "symbol":
    case "text":
      return [e.at];
    default:
      return [];
  }
}

export type SnapKind = "endpoint" | "midpoint" | "perpendicular" | "grid" | "none";

export interface Snap {
  point: Pt;
  kind: SnapKind;
}

export interface SnapOptions {
  /** How close the cursor must be to a snap candidate (m, from pixels at the current zoom). */
  tolerance: number;
  /** Grid spacing (m); 0 turns grid snapping off. */
  grid: number;
  /** The point the current line starts from, for perpendicular snapping. */
  from: Pt | null;
  endpoint: boolean;
  midpoint: boolean;
  perpendicular: boolean;
}

/**
 * Where a click at `p` lands. The nearest candidate within tolerance wins, by priority:
 * endpoint, then midpoint, then perpendicular (the foot of the perpendicular from `from`
 * onto a nearby segment), then the grid.
 */
export function snap(p: Pt, entities: Entity[], o: SnapOptions): Snap {
  const nearest = (cands: Pt[]): Pt | null => {
    let best: Pt | null = null;
    let bd = o.tolerance;
    for (const c of cands) {
      const d = dist(p, c);
      if (d <= bd) {
        bd = d;
        best = c;
      }
    }
    return best;
  };
  if (o.endpoint) {
    const e = nearest(entities.flatMap(endpoints));
    if (e) return { point: e, kind: "endpoint" };
  }
  const segs = entities.flatMap(segments);
  if (o.midpoint) {
    const m = nearest(segs.map(([a, b]) => mid(a, b)));
    if (m) return { point: m, kind: "midpoint" };
  }
  if (o.perpendicular && o.from) {
    const from = o.from;
    const feet = segs
      .map(([a, b]) => {
        const f = foot(from, a, b);
        // Only onto the segment itself.
        return f && dist(a, f) + dist(f, b) <= dist(a, b) * (1 + 1e-9) ? f : null;
      })
      .filter((f): f is Pt => f !== null);
    const f = nearest(feet);
    if (f) return { point: f, kind: "perpendicular" };
  }
  if (o.grid > 0) {
    return {
      point: [Math.round(p[0] / o.grid) * o.grid, Math.round(p[1] / o.grid) * o.grid],
      kind: "grid",
    };
  }
  return { point: p, kind: "none" };
}

/** Length of an entity's drawn path (m): lines, polylines, arcs. */
export function length(e: Entity): number {
  if (e.kind === "arc") {
    let sweep = e.end - e.start;
    while (sweep < 0) sweep += 2 * Math.PI;
    return e.radius * sweep;
  }
  return segments(e).reduce((s, [a, b]) => s + dist(a, b), 0);
}

/** Ids of the reference points that measured points were taken from. */
export function measuredFrom(entities: Entity[]): Set<string> {
  const ids = new Set<string>();
  for (const e of entities)
    if (e.kind === "point" && e.measurement)
      for (const id of e.measurement.method === "baseline_offset"
        ? [e.measurement.from, e.measurement.to]
        : e.measurement.refs.map((r) => r.point))
        ids.add(id);
  return ids;
}

/**
 * Whether an entity can be dragged to a new place. Measured points sit where their field
 * measurements put them, the reference points they were taken from stay where they were
 * (or the readings would no longer give the point), and underlays sit where their
 * calibration or slice placed them.
 */
export const movable = (e: Entity, all: Entity[]): boolean =>
  e.kind !== "underlay" && !(e.kind === "point" && e.measurement) && !measuredFrom(all).has(e.id);

/** An entity moved by `d` metres; built items keep their parameters and geometry. */
export function translate(e: Entity, d: Pt): Entity {
  const m = (p: Pt) => add(p, d);
  switch (e.kind) {
    case "line":
    case "dimension":
      return { ...e, a: m(e.a), b: m(e.b) };
    case "polyline":
      return { ...e, points: e.points.map(m) };
    case "arc":
      return { ...e, center: m(e.center) };
    case "room":
    case "road": {
      const geometry = {
        segments: e.geometry.segments.map((s) => ({ ...s, a: m(s.a), b: m(s.b) })),
        arcs: e.geometry.arcs.map((a) => ({ ...a, center: m(a.center) })),
      };
      return e.kind === "room"
        ? { ...e, outline: e.outline.map(m), geometry }
        : { ...e, centreline: e.centreline.map(m), geometry };
    }
    case "underlay":
      return e;
    default:
      return { ...e, at: m(e.at) };
  }
}

/**
 * Clicked corners without repeats: a double-click lands twice on one spot, and clicking the
 * first corner again to close a room repeats it; either gives a zero-length wall.
 */
export function corners(pts: Pt[], closed: boolean, tol = 1e-3): Pt[] {
  const out = pts.filter((p, i) => i === 0 || dist(p, pts[i - 1]) > tol);
  if (closed) while (out.length > 1 && dist(out[out.length - 1], out[0]) <= tol) out.pop();
  return out;
}

/** Distance from `p` to an entity, for picking (m). */
function distanceTo(e: Entity, p: Pt): number {
  let d = Infinity;
  for (const [a, b] of segments(e)) {
    const f = foot(p, a, b);
    const on = f && dist(a, f) + dist(f, b) <= dist(a, b) * (1 + 1e-9);
    d = Math.min(d, on && f ? dist(p, f) : Math.min(dist(p, a), dist(p, b)));
  }
  if (e.kind === "arc") d = Math.min(d, Math.abs(dist(p, e.center) - e.radius));
  for (const q of endpoints(e)) d = Math.min(d, dist(p, q));
  if (e.kind === "north" || e.kind === "scalebar" || e.kind === "legend")
    d = Math.min(d, dist(p, e.at));
  return d;
}

/** Items placed at a point. Snapping often puts them on a wall or road edge. */
const AT_A_POINT = new Set<Entity["kind"]>([
  "point",
  "marker",
  "symbol",
  "text",
  "north",
  "scalebar",
  "legend",
]);

/**
 * The item a click at `p` means: the nearest within `reach` (m), except that an item placed at
 * a point wins over lines. Otherwise a marker snapped onto a road edge could never be picked.
 */
export function pickAt(entities: Entity[], p: Pt, reach: number): Entity | undefined {
  return entities
    .map((e) => [e, distanceTo(e, p), AT_A_POINT.has(e.kind) ? 0 : 1] as const)
    .filter(([, d]) => d <= reach)
    .sort((a, b) => a[2] - b[2] || a[1] - b[1])[0]?.[0];
}
