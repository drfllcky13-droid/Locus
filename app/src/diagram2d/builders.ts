// Room and roadway builders: parameters in, drawn geometry out. Pure. The geometry is stored
// in the document with the parameters, so the editor, the saved revision and the printed
// sheet all show the same lines. Project metres; polygons anticlockwise.
import { add, dist, scale, sub } from "./geometry";
import type { Pt } from "./model";

export interface Segment {
  a: Pt;
  b: Pt;
  /** Drawn dashed (lane dividers, a dashed centre line). */
  dashed: boolean;
}

export interface Arc {
  center: Pt;
  radius: number;
  /** Anticlockwise from start to end (radians). */
  start: number;
  end: number;
}

export interface Geometry {
  segments: Segment[];
  arcs: Arc[];
}

export interface Opening {
  kind: "door" | "window";
  /** Which wall: the edge from outline[wall] to outline[wall + 1]. */
  wall: number;
  /** Distance of the opening's near side from the wall's start (m, along the inner face). */
  at: number;
  width: number;
  /** Door hinge at the near or far side of the opening; the leaf opens into the room. */
  hinge: "near" | "far";
}

const unit = (v: Pt): Pt => scale(v, 1 / Math.hypot(v[0], v[1]));
/** Left normal of a direction (for an anticlockwise polygon, left is inside). */
const left = (u: Pt): Pt => [-u[1], u[0]];

/** Signed area; positive for anticlockwise. */
export function area(poly: Pt[]): number {
  let s = 0;
  for (let i = 0; i < poly.length; i++) {
    const [a, b] = [poly[i], poly[(i + 1) % poly.length]];
    s += a[0] * b[1] - b[0] * a[1];
  }
  return s / 2;
}

/** Where lines p + s·u and q + t·v meet (null if parallel). */
function meet(p: Pt, u: Pt, q: Pt, v: Pt): Pt | null {
  const den = u[0] * v[1] - u[1] * v[0];
  if (Math.abs(den) < 1e-12) return null;
  const s = ((q[0] - p[0]) * v[1] - (q[1] - p[1]) * v[0]) / den;
  return add(p, scale(u, s));
}

/**
 * Offset a polyline sideways by `d` (positive to the left of its direction), with mitred
 * joins. Closed polylines join their last edge to the first.
 */
export function offset(pts: Pt[], d: number, closed: boolean): Pt[] {
  const n = pts.length;
  const edges = closed ? n : n - 1;
  const lines: [Pt, Pt][] = [];
  for (let i = 0; i < edges; i++) {
    const [a, b] = [pts[i], pts[(i + 1) % n]];
    const u = unit(sub(b, a));
    lines.push([add(a, scale(left(u), d)), u]);
  }
  const out: Pt[] = [];
  for (let i = 0; i < n; i++) {
    const prev = closed ? lines[(i - 1 + edges) % edges] : i > 0 ? lines[i - 1] : null;
    const next = i < edges ? lines[i] : null;
    if (prev && next) {
      out.push(meet(prev[0], prev[1], next[0], next[1]) ?? add(pts[i], scale(left(next[1]), d)));
    } else if (next) {
      out.push(next[0]);
    } else if (prev) {
      out.push(add(pts[i], scale(left(prev[1]), d)));
    }
  }
  return out;
}

/** Split segment a→b into the parts outside the intervals `cuts` (distances from a). */
function cut(a: Pt, b: Pt, cuts: [number, number][]): Segment[] {
  const len = dist(a, b);
  const u = unit(sub(b, a));
  const sorted = [...cuts].sort((x, y) => x[0] - y[0]);
  const out: Segment[] = [];
  let from = 0;
  for (const [s, e] of sorted) {
    if (s > from)
      out.push({ a: add(a, scale(u, from)), b: add(a, scale(u, Math.min(s, len))), dashed: false });
    from = Math.max(from, e);
  }
  if (from < len) out.push({ a: add(a, scale(u, from)), b, dashed: false });
  return out;
}

/**
 * A room from its inner outline (any orientation; made anticlockwise), wall thickness and
 * openings. Walls are drawn as inner and outer faces; each opening leaves a gap with jambs; a
 * door gets its leaf and swing, a window a pane line in the middle of the wall.
 */
export function room(outline: Pt[], thickness: number, openings: Opening[]): Geometry {
  const inner = area(outline) < 0 ? [...outline].reverse() : outline;
  // Openings are given against the outline as entered; follow the reversal.
  const flipped = inner !== outline;
  const n = inner.length;
  const outer = offset(inner, -thickness, true);
  const segments: Segment[] = [];
  const arcs: Arc[] = [];
  for (let w = 0; w < n; w++) {
    const [a, b] = [inner[w], inner[(w + 1) % n]];
    const [oa, ob] = [outer[w], outer[(w + 1) % n]];
    const len = dist(a, b);
    const u = unit(sub(b, a));
    const out = scale(left(u), -1); // toward the outside
    const here = openings
      .filter((o) => (flipped ? n - 2 - o.wall + (o.wall === n - 1 ? n : 0) : o.wall) % n === w)
      .map(
        (o) =>
          (flipped
            ? { ...o, at: len - o.at - o.width, hinge: o.hinge === "near" ? "far" : "near" }
            : o) as Opening,
      )
      .filter((o) => o.at >= 0 && o.at + o.width <= len + 1e-9);
    segments.push(
      ...cut(
        a,
        b,
        here.map((o) => [o.at, o.at + o.width]),
      ),
    );
    // The outer face, cut at the same places measured square across the wall.
    const t0 = (oa[0] - a[0]) * u[0] + (oa[1] - a[1]) * u[1];
    const outerCuts = here.map((o) => [o.at - t0, o.at + o.width - t0] as [number, number]);
    segments.push(...cut(oa, ob, outerCuts));
    for (const o of here) {
      const p0 = add(a, scale(u, o.at));
      const p1 = add(a, scale(u, o.at + o.width));
      // Jambs across the wall.
      segments.push({ a: p0, b: add(p0, scale(out, thickness)), dashed: false });
      segments.push({ a: p1, b: add(p1, scale(out, thickness)), dashed: false });
      if (o.kind === "window") {
        const m = scale(out, thickness / 2);
        segments.push({ a: add(p0, m), b: add(p1, m), dashed: false });
      } else {
        // Leaf open at 90° into the room, from the hinge; the swing is a quarter circle.
        const [hinge, free] = o.hinge === "near" ? [p0, p1] : [p1, p0];
        const inward = left(u);
        const leafEnd = add(hinge, scale(inward, o.width));
        segments.push({ a: hinge, b: leafEnd, dashed: false });
        const ang = (p: Pt) => Math.atan2(p[1] - hinge[1], p[0] - hinge[0]);
        const [s, e] = [ang(free), ang(leafEnd)];
        // Anticlockwise from whichever end makes the quarter turn.
        let sweep = e - s;
        while (sweep < 0) sweep += 2 * Math.PI;
        arcs.push(
          sweep <= Math.PI
            ? { center: hinge, radius: o.width, start: s, end: e }
            : { center: hinge, radius: o.width, start: e, end: s },
        );
      }
    }
  }
  return { segments, arcs };
}

export interface RoadParams {
  /** Lanes on each side of the centreline: [left, right] of its direction. */
  lanes: [number, number];
  laneWidth: number;
  /** Paved shoulder beyond the edge line on each side (m). */
  shoulder: number;
  centre: "dashed" | "solid" | "double" | "none";
  /** Radius of the centreline's curves at its corners (m); 0 keeps sharp corners. */
  radius?: number;
}

/**
 * Round a polyline's corners with circular arcs of radius `r`, tangent to both edges, drawn
 * as chords of at most 2°. A corner whose edges are too short for the full radius gets the
 * largest arc that fits (tangent points at most halfway along each edge).
 */
export function fillet(pts: Pt[], r: number): Pt[] {
  if (r <= 0 || pts.length < 3) return pts;
  const out: Pt[] = [pts[0]];
  for (let i = 1; i < pts.length - 1; i++) {
    const [a, v, b] = [pts[i - 1], pts[i], pts[i + 1]];
    const [u1, u2] = [unit(sub(v, a)), unit(sub(b, v))];
    const turn = Math.atan2(u1[0] * u2[1] - u1[1] * u2[0], u1[0] * u2[0] + u1[1] * u2[1]);
    if (Math.abs(turn) < 1e-9) {
      out.push(v);
      continue;
    }
    const half = Math.tan(Math.abs(turn) / 2);
    const t = Math.min(r * half, dist(a, v) / 2, dist(v, b) / 2);
    const rr = t / half;
    const side = Math.sign(turn); // +1 turning left: the centre is on the left
    const p1 = add(v, scale(u1, -t));
    const centre = add(p1, scale(left(u1), side * rr));
    const start = Math.atan2(p1[1] - centre[1], p1[0] - centre[0]);
    const steps = Math.max(1, Math.ceil(Math.abs(turn) / ((2 * Math.PI) / 180)));
    for (let k = 0; k <= steps; k++) {
      const ang = start + (turn * k) / steps;
      out.push([centre[0] + rr * Math.cos(ang), centre[1] + rr * Math.sin(ang)]);
    }
  }
  out.push(pts[pts.length - 1]);
  return out;
}

/** Broken-line markings: 3 m painted, 9 m gap (a common lane-line pattern). */
export const DASH: [number, number] = [3, 9];

/** The painted pieces of a dashed line along a polyline; the pattern runs on round corners. */
export function dashes(pts: Pt[], [on, off]: [number, number] = DASH): Segment[] {
  const out: Segment[] = [];
  let phase = 0; // distance into the current on+off cycle
  for (let i = 1; i < pts.length; i++) {
    const [a, b] = [pts[i - 1], pts[i]];
    const len = dist(a, b);
    const u = unit(sub(b, a));
    let t = 0;
    while (t < len) {
      const step = phase < on ? Math.min(on - phase, len - t) : Math.min(on + off - phase, len - t);
      if (phase < on)
        out.push({ a: add(a, scale(u, t)), b: add(a, scale(u, t + step)), dashed: true });
      t += step;
      phase = (phase + step) % (on + off);
    }
  }
  return out;
}

/**
 * A road along a centreline: the centre marking, dashed lane dividers, solid edge lines, and
 * the outer edges of the shoulders. Dashed markings are drawn as their painted pieces.
 */
export function road(drawn: Pt[], p: RoadParams): Geometry {
  const centreline = fillet(drawn, p.radius ?? 0);
  const segments: Segment[] = [];
  const line = (pts: Pt[], dashed: boolean) => {
    if (dashed) segments.push(...dashes(pts));
    else for (let i = 1; i < pts.length; i++) segments.push({ a: pts[i - 1], b: pts[i], dashed });
  };
  if (p.centre === "double") {
    line(offset(centreline, 0.1, false), false);
    line(offset(centreline, -0.1, false), false);
  } else if (p.centre !== "none") {
    line(centreline, p.centre === "dashed");
  }
  for (const [side, n] of [
    [1, p.lanes[0]],
    [-1, p.lanes[1]],
  ] as const) {
    for (let k = 1; k < n; k++) line(offset(centreline, side * k * p.laneWidth, false), true);
    const edge = side * n * p.laneWidth;
    line(offset(centreline, edge, false), false);
    if (p.shoulder > 0) line(offset(centreline, edge + side * p.shoulder, false), false);
  }
  return { segments, arcs: [] };
}
