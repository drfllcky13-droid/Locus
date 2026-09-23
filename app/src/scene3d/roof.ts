// Roofs over a room's outer outline. Pure.
//
// Flat roofs fit any outline. Shed, gable and hip roofs need a rectangular outline (four
// corners, square within 0.5°); a hip roof on any other shape needs a straight skeleton,
// which is not built (see docs/DECISIONS.md). Every sloped plane passes through the wall
// line at the eaves height, so the roof sits on the walls exactly; the overhang continues
// the slope outward and down.
import { ShapeUtils, Vector2 } from "three";
import type { Pt } from "../diagram2d/model";
import type { Mesh } from "./extrude";

export type RoofType = "flat" | "shed" | "gable" | "hip";

export interface RoofParams {
  type: RoofType;
  /** Height of the wall tops the roof sits on (m, project z). */
  eaves: number;
  /** Slope, degrees (ignored for flat). */
  pitch: number;
  /** Horizontal overhang past the walls (m). */
  overhang: number;
  /** Slab thickness of a flat roof (m). */
  thickness: number;
}

export const DEFAULT_ROOF: RoofParams = {
  type: "gable",
  eaves: 2.6,
  pitch: 30,
  overhang: 0.3,
  thickness: 0.2,
};

export interface Rect {
  centre: Pt;
  /** Along the long side, and across it (unit vectors). */
  u: Pt;
  v: Pt;
  length: number;
  width: number;
}

const sub = (a: Pt, b: Pt): Pt => [a[0] - b[0], a[1] - b[1]];
const dot = (a: Pt, b: Pt) => a[0] * b[0] + a[1] * b[1];
const len = (a: Pt) => Math.hypot(a[0], a[1]);

/** The outline as a rectangle, or null if it isn't one (four corners square within 0.5°). */
export function rectangle(outline: Pt[]): Rect | null {
  if (outline.length !== 4) return null;
  for (let i = 0; i < 4; i++) {
    const a = sub(outline[(i + 1) % 4], outline[i]);
    const b = sub(outline[(i + 2) % 4], outline[(i + 1) % 4]);
    const cos = dot(a, b) / (len(a) * len(b));
    if (Math.abs(cos) > Math.sin((0.5 * Math.PI) / 180)) return null;
  }
  const e0 = sub(outline[1], outline[0]);
  const e1 = sub(outline[2], outline[1]);
  const [long, short] = len(e0) >= len(e1) ? [e0, e1] : [e1, e0];
  const centre: Pt = [
    outline.reduce((s, p) => s + p[0], 0) / 4,
    outline.reduce((s, p) => s + p[1], 0) / 4,
  ];
  // Average opposite sides so a slightly skewed survey doesn't bias the size.
  const L =
    (len(long) +
      len(len(e0) >= len(e1) ? sub(outline[2], outline[3]) : sub(outline[3], outline[0]))) /
    2;
  const W =
    (len(short) +
      len(len(e0) >= len(e1) ? sub(outline[3], outline[0]) : sub(outline[2], outline[3]))) /
    2;
  const u: Pt = [long[0] / len(long), long[1] / len(long)];
  return { centre, u, v: [-u[1], u[0]], length: L, width: W };
}

function vertex(m: Mesh, p: Pt, z: number) {
  m.positions.push(p[0], p[1], z);
  return m.positions.length / 3 - 1;
}

/** A planar polygon (in order) as a triangle fan; fine for the convex pieces used here. */
function face(m: Mesh, pts: [Pt, number][]) {
  const v = pts.map(([p, z]) => vertex(m, p, z));
  for (let i = 1; i < v.length - 1; i++) m.indices.push(v[0], v[i], v[i + 1]);
}

/** Point at local (s along u, t along v) of a rectangle. */
const at = (r: Rect, s: number, t: number): Pt => [
  r.centre[0] + s * r.u[0] + t * r.v[0],
  r.centre[1] + s * r.u[1] + t * r.v[1],
];

/** A roof over `outline` (the walls' outer face). Throws if the type needs a rectangle. */
export function roof(outline: Pt[], p: RoofParams): Mesh {
  const m: Mesh = { positions: [], indices: [] };
  if (p.type === "flat") {
    const c = outline.map((q) => new Vector2(q[0], q[1]));
    // Grow the outline by the overhang (mitred), then a slab from eaves up.
    const n = outline.length;
    const ccw = !ShapeUtils.isClockWise(c);
    const grown = outline.map((q, i) => {
      const a = outline[(i - 1 + n) % n];
      const b = outline[(i + 1) % n];
      const n1 = normalOut(a, q, ccw);
      const n2 = normalOut(q, b, ccw);
      const bis: Pt = [n1[0] + n2[0], n1[1] + n2[1]];
      const k = p.overhang / (1 + dot(n1, n2)) || 0;
      return [q[0] + bis[0] * k, q[1] + bis[1] * k] as Pt;
    });
    const tris = ShapeUtils.triangulateShape(
      grown.map((q) => new Vector2(q[0], q[1])),
      [],
    );
    const top = grown.map((q) => vertex(m, q, p.eaves + p.thickness));
    const bot = grown.map((q) => vertex(m, q, p.eaves));
    for (const [a, b, c2] of tris) m.indices.push(top[a], top[b], top[c2], bot[a], bot[c2], bot[b]);
    for (let i = 0; i < n; i++) {
      const j = (i + 1) % n;
      m.indices.push(bot[i], bot[j], top[j], bot[i], top[j], top[i]);
    }
    return m;
  }
  const r = rectangle(outline);
  if (!r) throw new Error(`A ${p.type} roof needs a rectangular room; use a flat roof here.`);
  const k = Math.tan((p.pitch * Math.PI) / 180);
  const [hl, hw, o] = [r.length / 2, r.width / 2, p.overhang];
  // Height of a plane sloping up from the wall line at distance d inward.
  const z = (d: number) => p.eaves + d * k;
  if (p.type === "shed") {
    // Rises across the width, from the -v wall to the +v wall.
    face(m, [
      [at(r, -hl - o, -hw - o), z(-o)],
      [at(r, hl + o, -hw - o), z(-o)],
      [at(r, hl + o, hw + o), z(2 * hw + o)],
      [at(r, -hl - o, hw + o), z(2 * hw + o)],
    ]);
    // Walls up to the roof at the ends and the high side.
    for (const s of [-1, 1])
      face(m, [
        [at(r, s * hl, -hw), p.eaves],
        [at(r, s * hl, hw), p.eaves],
        [at(r, s * hl, hw), z(2 * hw)],
        [at(r, s * hl, -hw), z(0)],
      ]);
    face(m, [
      [at(r, -hl, hw), p.eaves],
      [at(r, hl, hw), p.eaves],
      [at(r, hl, hw), z(2 * hw)],
      [at(r, -hl, hw), z(2 * hw)],
    ]);
    return m;
  }
  const ridge = z(hw);
  if (p.type === "gable") {
    for (const s of [-1, 1]) {
      face(m, [
        [at(r, -hl - o, s * (hw + o)), z(-o)],
        [at(r, hl + o, s * (hw + o)), z(-o)],
        [at(r, hl + o, 0), ridge],
        [at(r, -hl - o, 0), ridge],
      ]);
      // Gable end wall, on the wall line.
      face(m, [
        [at(r, s * hl, -hw), p.eaves],
        [at(r, s * hl, hw), p.eaves],
        [at(r, s * hl, 0), ridge],
      ]);
    }
    return m;
  }
  // Hip: the ridge is shorter than the building by the width (45° in plan at the corners).
  const rl = Math.max(0, hl - hw);
  for (const s of [-1, 1]) {
    face(m, [
      [at(r, -hl - o, s * (hw + o)), z(-o)],
      [at(r, hl + o, s * (hw + o)), z(-o)],
      [at(r, rl, 0), ridge],
      [at(r, -rl, 0), ridge],
    ]);
    face(m, [
      [at(r, s * (hl + o), -hw - o), z(-o)],
      [at(r, s * (hl + o), hw + o), z(-o)],
      [at(r, s * rl, 0), ridge],
    ]);
  }
  return m;
}

function normalOut(a: Pt, b: Pt, ccw: boolean): Pt {
  const d = sub(b, a);
  const l = len(d);
  // Right of the direction is outside for an anticlockwise outline.
  return ccw ? [d[1] / l, -d[0] / l] : [-d[1] / l, d[0] / l];
}
