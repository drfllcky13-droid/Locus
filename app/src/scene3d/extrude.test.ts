import { describe, expect, it } from "vitest";
import { road, room, type Opening, type RoadParams, type Segment } from "../diagram2d/builders";
import type { Pt } from "../diagram2d/model";
import {
  DEFAULT_EXTRUDE,
  MARKING_LIFT,
  relative,
  roadSolids,
  roomSolids,
  type Mesh,
} from "./extrude";

const vertices = (m: Mesh) => {
  const v: [number, number, number][] = [];
  for (let i = 0; i < m.positions.length; i += 3)
    v.push([m.positions[i], m.positions[i + 1], m.positions[i + 2]]);
  return v;
};

/** Distance from p to segment ab in plan. */
function toSegment(p: Pt, a: Pt, b: Pt): number {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
  const L = dx * dx + dy * dy;
  const t = L ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / L)) : 0;
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
}
const nearest = (p: Pt, segs: Segment[]) => Math.min(...segs.map((s) => toSegment(p, s.a, s.b)));

// An irregular room far from the origin (a real project frame), with a door and a window.
const O: Pt = [431_200.25, 5_390_110.75];
const outline: Pt[] = [
  [0, 0],
  [6.2, 0],
  [6.9, 4.1],
  [0.3, 4.6],
].map(([x, y]) => [O[0] + x, O[1] + y]);
const openings: Opening[] = [
  { kind: "door", wall: 0, at: 1.0, width: 0.9, hinge: "near" },
  { kind: "window", wall: 2, at: 2.0, width: 1.5, hinge: "near", sill: 1.0, head: 2.2 },
];
const T = 0.23;
const P = { ...DEFAULT_EXTRUDE, base: 212.4, wallHeight: 2.7 };

describe("room extrusion", () => {
  const plan = room(outline, T, openings);
  const { wall } = roomSolids(outline, T, openings, P);
  const v = vertices(wall);

  it("puts every wall vertex on the plan's lines (acceptance: within 1 mm; here 1e-9 m)", () => {
    // The plan's wall lines: faces and jambs (door leaf, swing and window pane excluded).
    const bare = room(outline, T, []).segments;
    const jambsAndFaces = plan.segments.filter(
      (s) => nearest(s.a, bare) < 1e-9 || nearest(s.b, bare) < 1e-9,
    );
    let worst = 0;
    for (const [x, y] of v) worst = Math.max(worst, nearest([x, y], jambsAndFaces));
    expect(worst).toBeLessThan(1e-9);
  });

  it("has a vertex at every end of the plan's wall faces and jambs", () => {
    const bare = room(outline, T, []).segments;
    for (const s of plan.segments) {
      for (const q of [s.a, s.b]) {
        if (nearest(q, bare) > 1e-9 && !plan.segments.some((j) => isJamb(j, q))) continue;
        const d = Math.min(...v.map(([x, y]) => Math.hypot(x - q[0], y - q[1])));
        expect(d).toBeLessThan(1e-9);
      }
    }
  });

  it("leaves the door open to its head and the window between sill and head", () => {
    const zs = (inside: (p: Pt) => boolean) =>
      v.filter(([x, y]) => inside([x, y])).map(([, , z]) => z - P.base);
    const along = (w: number, p: Pt) => {
      const [a, b] = [outline[w], outline[(w + 1) % outline.length]];
      const L = Math.hypot(b[0] - a[0], b[1] - a[1]);
      return ((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / L;
    };
    // Strictly inside the door's width: only the lintel's vertices (z ≥ 2.1).
    const door = zs((p) => {
      const t = along(0, p);
      return t > 1.0 + 1e-6 && t < 1.9 - 1e-6 && Math.abs(p[1] - O[1]) < 0.5;
    });
    expect(door.every((z) => z >= 2.1 - 1e-9)).toBe(true);
    const all = v.map(([, , z]) => z - P.base);
    expect(Math.min(...all)).toBeCloseTo(0, 12);
    expect(Math.max(...all)).toBeCloseTo(2.7, 12);
    // The window's opening: vertices at 1.0 and 2.2 exist.
    expect(all.some((z) => Math.abs(z - 1.0) < 1e-9)).toBe(true);
    expect(all.some((z) => Math.abs(z - 2.2) < 1e-9)).toBe(true);
  });

  it("stays within 1 mm after moving to a render origin in f32", () => {
    const origin: [number, number, number] = [O[0] + 3, O[1] + 2, P.base];
    const f = relative(wall, origin);
    let worst = 0;
    for (let i = 0; i < f.length; i++)
      worst = Math.max(worst, Math.abs(f[i] + origin[i % 3] - wall.positions[i]));
    expect(worst).toBeLessThan(1e-6);
  });
});

function isJamb(j: Segment, q: Pt) {
  const L = Math.hypot(j.b[0] - j.a[0], j.b[1] - j.a[1]);
  return (
    Math.abs(L - T) < 1e-9 &&
    (Math.hypot(j.a[0] - q[0], j.a[1] - q[1]) < 1e-12 ||
      Math.hypot(j.b[0] - q[0], j.b[1] - q[1]) < 1e-12)
  );
}

describe("road extrusion", () => {
  const cl: Pt[] = [
    [O[0], O[1]],
    [O[0] + 40, O[1]],
    [O[0] + 70, O[1] + 25],
  ];
  const r: RoadParams = {
    lanes: [2, 1],
    laneWidth: 3.5,
    shoulder: 1.2,
    centre: "dashed",
    radius: 15,
  };
  const plan = road(cl, r).segments;
  const s = roadSolids(cl, r, P);

  it("puts the road and shoulder edges on the plan's lines", () => {
    let worst = 0;
    for (const m of [s.road, s.shoulder])
      for (const [x, y, z] of vertices(m)) {
        worst = Math.max(worst, nearest([x, y], plan));
        expect(z).toBe(P.base);
      }
    expect(worst).toBeLessThan(1e-9);
  });

  it("centres each marking on its plan line, just above the road", () => {
    const hw = P.lineWidth / 2;
    let worst = 0;
    for (const [x, y, z] of vertices(s.marking)) {
      worst = Math.max(worst, Math.abs(nearest([x, y], plan) - hw));
      expect(z).toBeCloseTo(P.base + MARKING_LIFT, 12);
    }
    // On curves the strip's corner can sit a little nearer another piece of the same line
    // than hw; everything stays within a tenth of a millimetre.
    expect(worst).toBeLessThan(1e-4);
  });
});
