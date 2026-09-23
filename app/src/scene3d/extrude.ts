// Diagram to 3D: rooms become walls, roads become surfaces with their markings. Pure.
// Plan x, y are the diagram's own coordinates, computed by the same functions that draw the
// plan (walls, roadLines in diagram2d/builders.ts), so the 3D and the 2D coincide exactly;
// z is the base elevation plus heights. Project metres, f64 throughout.
import {
  DASH,
  DOOR_HEAD,
  WINDOW_HEAD,
  WINDOW_SILL,
  dashes,
  fillet,
  offset,
  roadLines,
  walls,
  type Opening,
  type RoadParams,
} from "../diagram2d/builders";
import type { Diagram, Pt } from "../diagram2d/model";

/** Triangles: flat xyz triplets and indices into them. */
export interface Mesh {
  positions: number[];
  indices: number[];
}

export type Part = "wall" | "floor" | "road" | "shoulder" | "marking";

export interface ExtrudeParams {
  /** Floor or road surface elevation (m, project z). */
  base: number;
  /** Wall height above the base (m). */
  wallHeight: number;
  /** Floor slab under each room (m thick; 0 for none). */
  floor: number;
  /** Painted line width (m). */
  lineWidth: number;
}

export const DEFAULT_EXTRUDE: ExtrudeParams = {
  base: 0,
  wallHeight: 2.6,
  floor: 0,
  lineWidth: 0.1,
};

/** Markings sit this far above the road so they don't fight it in the depth buffer (m). */
export const MARKING_LIFT = 0.003;

function mesh(): Mesh {
  return { positions: [], indices: [] };
}

function vertex(m: Mesh, [x, y]: Pt, z: number): number {
  m.positions.push(x, y, z);
  return m.positions.length / 3 - 1;
}

/** A prism over a convex quadrilateral footprint (in order around), from z0 to z1. */
function prism(m: Mesh, quad: Pt[], z0: number, z1: number) {
  const bottom = quad.map((p) => vertex(m, p, z0));
  const top = quad.map((p) => vertex(m, p, z1));
  // Orientation-independent: both windings for caps would double the triangles, so orient
  // the footprint anticlockwise first.
  const ccw = signedArea(quad) > 0;
  const [b, t] = ccw ? [bottom, top] : [[...bottom].reverse(), [...top].reverse()];
  m.indices.push(t[0], t[1], t[2], t[0], t[2], t[3]); // top, facing up
  m.indices.push(b[0], b[2], b[1], b[0], b[3], b[2]); // bottom, facing down
  for (let i = 0; i < 4; i++) {
    const j = (i + 1) % 4;
    m.indices.push(b[i], b[j], t[j], b[i], t[j], t[i]); // sides, facing out
  }
}

function signedArea(poly: Pt[]): number {
  let s = 0;
  for (let i = 0; i < poly.length; i++) {
    const [a, b] = [poly[i], poly[(i + 1) % poly.length]];
    s += a[0] * b[1] - b[0] * a[1];
  }
  return s / 2;
}

/** A strip between two polylines with the same number of points, at height z. */
function strip(m: Mesh, left: Pt[], right: Pt[], z: number) {
  for (let i = 1; i < left.length; i++) {
    const [a, b, c, d] = [left[i - 1], right[i - 1], right[i], left[i]];
    const quad = [vertex(m, a, z), vertex(m, b, z), vertex(m, c, z), vertex(m, d, z)];
    // Face up whichever side `left` is on.
    const up = (b[0] - a[0]) * (d[1] - a[1]) - (b[1] - a[1]) * (d[0] - a[0]) > 0;
    if (up) m.indices.push(quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]);
    else m.indices.push(quad[0], quad[2], quad[1], quad[0], quad[3], quad[2]);
  }
}

const add = (a: Pt, b: Pt): Pt => [a[0] + b[0], a[1] + b[1]];
const mul = (a: Pt, k: number): Pt => [a[0] * k, a[1] * k];

/** Height ranges of wall left solid over an opening (above and below it), within 0..h. */
function around(o: Opening, h: number): [number, number][] {
  const head = Math.min(o.head ?? (o.kind === "door" ? DOOR_HEAD : WINDOW_HEAD), h);
  const sill = o.kind === "door" ? 0 : Math.max(0, Math.min(o.sill ?? WINDOW_SILL, head));
  const out: [number, number][] = [];
  if (sill > 0) out.push([0, sill]);
  if (head < h) out.push([head, h]);
  return out;
}

/**
 * A room's walls as solids. Along each wall, full-height pieces run between the openings, and
 * over each opening the wall is kept above its head (and, for a window, below its sill). The
 * pieces' footprints are the plan's own: inner face, outer face (mitred at the corners), and
 * jambs square across the wall.
 */
export function roomSolids(
  outline: Pt[],
  thickness: number,
  openings: Opening[],
  p: ExtrudeParams,
): { wall: Mesh; floor: Mesh } {
  const wall = mesh();
  const floor = mesh();
  const h = p.wallHeight;
  for (const w of walls(outline, thickness, openings)) {
    // Footprint of the wall between distances s and e along the inner face.
    const quad = (s: number, e: number): Pt[] => [
      add(w.a, mul(w.u, s)),
      add(w.a, mul(w.u, e)),
      e >= w.len ? w.ob : add(add(w.a, mul(w.u, e)), mul(w.out, thickness)),
      s <= 0 ? w.oa : add(add(w.a, mul(w.u, s)), mul(w.out, thickness)),
    ];
    const ops = [...w.openings].sort((x, y) => x.at - y.at);
    let from = 0;
    for (const o of ops) {
      if (o.at > from) prism(wall, quad(from, o.at), p.base, p.base + h);
      for (const [z0, z1] of around(o, h))
        prism(wall, quad(o.at, o.at + o.width), p.base + z0, p.base + z1);
      from = Math.max(from, o.at + o.width);
    }
    if (from < w.len) prism(wall, quad(from, w.len), p.base, p.base + h);
  }
  if (p.floor > 0) {
    // The slab under the room and its walls: the outer outline, fanned from its first corner
    // (fine for the convex and mildly concave rooms the builder is for).
    const outer = walls(outline, thickness, []).map((w) => w.oa);
    const top = outer.map((q) => vertex(floor, q, p.base));
    const bot = outer.map((q) => vertex(floor, q, p.base - p.floor));
    for (let i = 1; i < outer.length - 1; i++) {
      floor.indices.push(top[0], top[i], top[i + 1], bot[0], bot[i + 1], bot[i]);
    }
    for (let i = 0; i < outer.length; i++) {
      const j = (i + 1) % outer.length;
      floor.indices.push(bot[i], bot[j], top[j], bot[i], top[j], top[i]);
    }
  }
  return { wall, floor };
}

/**
 * A road's surfaces and markings. Lanes run between the outer edge lines, shoulders beyond
 * them; each painted line becomes a strip `lineWidth` wide centred on the plan's line (broken
 * lines as their painted pieces), just above the surface.
 */
export function roadSolids(
  centreline: Pt[],
  r: RoadParams,
  p: ExtrudeParams,
): { road: Mesh; shoulder: Mesh; marking: Mesh } {
  const road = mesh();
  const shoulder = mesh();
  const marking = mesh();
  const lines = roadLines(centreline, r);
  // The rounded centreline: the offset-0 base every line was built from.
  const base = fillet(centreline, r.radius ?? 0);
  const at = (d: number) => (d === 0 ? base : offset(base, d, false));
  const [left, right] = [r.lanes[0] * r.laneWidth, -r.lanes[1] * r.laneWidth];
  if (left > right) strip(road, at(left), at(right), p.base);
  if (r.shoulder > 0) {
    strip(shoulder, at(left + r.shoulder), at(left), p.base);
    strip(shoulder, at(right), at(right - r.shoulder), p.base);
  }
  const hw = p.lineWidth / 2;
  const z = p.base + MARKING_LIFT;
  for (const l of lines) {
    if (!l.painted) continue;
    const pieces = l.dashed ? dashes(l.pts, r.dash ?? DASH).map((s) => [s.a, s.b]) : [l.pts];
    for (const pts of pieces) strip(marking, offset(pts, hw, false), offset(pts, -hw, false), z);
  }
  return { road, shoulder, marking };
}

/** Every room and road of a diagram (on visible layers), as meshes by part. */
export function extrude(d: Diagram, p: ExtrudeParams): Record<Part, Mesh[]> {
  const out: Record<Part, Mesh[]> = { wall: [], floor: [], road: [], shoulder: [], marking: [] };
  const hidden = new Set(d.layers.filter((l) => !l.visible).map((l) => l.id));
  for (const e of d.entities) {
    if (hidden.has(e.layer)) continue;
    if (e.kind === "room") {
      const s = roomSolids(e.outline, e.thickness, e.openings, p);
      out.wall.push(s.wall);
      if (s.floor.indices.length) out.floor.push(s.floor);
    } else if (e.kind === "road") {
      const s = roadSolids(e.centreline, e.road, p);
      out.road.push(s.road);
      if (s.shoulder.indices.length) out.shoulder.push(s.shoulder);
      out.marking.push(s.marking);
    }
  }
  return out;
}

/**
 * Positions relative to the render origin as f32 for the GPU. The subtraction happens in f64,
 * so only the (small) remainder is rounded.
 */
export function relative(m: Mesh, origin: [number, number, number]): Float32Array {
  const out = new Float32Array(m.positions.length);
  for (let i = 0; i < m.positions.length; i++) out[i] = m.positions[i] - origin[i % 3];
  return out;
}
