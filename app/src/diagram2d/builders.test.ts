import { describe, expect, it } from "vitest";
import { area, dashes, fillet, offset, road, room, type Segment } from "./builders";
import { dist } from "./geometry";
import type { Pt } from "./model";

const close = (a: Pt, b: Pt) => dist(a, b) < 1e-9;
const total = (s: Segment[]) => s.reduce((t, x) => t + dist(x.a, x.b), 0);

describe("offset", () => {
  it("mitres the corners of a closed polygon", () => {
    const sq: Pt[] = [
      [0, 0],
      [4, 0],
      [4, 3],
      [0, 3],
    ];
    expect(area(sq)).toBe(12);
    // Anticlockwise: negative offset goes outside.
    const out = offset(sq, -0.2, true);
    expect(close(out[0], [-0.2, -0.2])).toBe(true);
    expect(close(out[2], [4.2, 3.2])).toBe(true);
  });

  it("keeps a straight open line parallel", () => {
    const o = offset(
      [
        [0, 0],
        [10, 0],
      ],
      3.5,
      false,
    );
    expect(o).toEqual([
      [0, 3.5],
      [10, 3.5],
    ]);
  });
});

describe("room", () => {
  const sq: Pt[] = [
    [0, 0],
    [4, 0],
    [4, 3],
    [0, 3],
  ];

  it("draws inner and outer wall faces", () => {
    const g = room(sq, 0.2, []);
    // Inner perimeter 14 m, outer (4.4 + 3.4) × 2 = 15.6 m.
    expect(total(g.segments)).toBeCloseTo(29.6, 9);
    expect(g.arcs).toEqual([]);
  });

  it("leaves a gap for a door, with jambs, leaf and swing", () => {
    const door = { kind: "door" as const, wall: 0, at: 1, width: 0.9, hinge: "near" as const };
    const g = room(sq, 0.2, [door]);
    // Both faces lose 0.9 m; two 0.2 m jambs and a 0.9 m leaf are added.
    expect(total(g.segments)).toBeCloseTo(29.6 - 1.8 + 0.4 + 0.9, 9);
    expect(g.arcs).toHaveLength(1);
    const arc = g.arcs[0];
    expect(close(arc.center, [1, 0])).toBe(true);
    expect(arc.radius).toBe(0.9);
    // A quarter turn, from the free jamb (1.9, 0) round to the open leaf (1, 0.9).
    expect(arc.start).toBeCloseTo(0, 12);
    expect(arc.end).toBeCloseTo(Math.PI / 2, 12);
  });

  it("puts a window pane in the middle of the wall", () => {
    const g = room(sq, 0.2, [{ kind: "window", wall: 1, at: 1, width: 1.2, hinge: "near" }]);
    const pane = g.segments.find(
      (s) => Math.abs(s.a[0] - 4.1) < 1e-9 && Math.abs(s.b[0] - 4.1) < 1e-9,
    );
    expect(pane && dist(pane.a, pane.b)).toBeCloseTo(1.2, 9);
  });

  it("accepts a clockwise outline", () => {
    const g = room([...sq].reverse(), 0.2, []);
    expect(total(g.segments)).toBeCloseTo(29.6, 9);
  });
});

describe("road", () => {
  it("draws markings, lane lines and edges at the right offsets", () => {
    const g = road(
      [
        [0, 0],
        [50, 0],
      ],
      { lanes: [2, 1], laneWidth: 3.5, shoulder: 1, centre: "dashed" },
    );
    const ys = [
      ...new Map(g.segments.map((s) => [s.a[1], [s.a[1], s.dashed] as const])).values(),
    ].sort((a, b) => a[0] - b[0]);
    expect(ys).toEqual([
      [-4.5, false], // right shoulder
      [-3.5, false], // right edge line
      [0, true], // dashed centre line
      [3.5, true], // lane divider on the left
      [7, false], // left edge line
      [8, false], // left shoulder
    ]);
  });

  it("paints dashes 3 m on, 9 m off, running on round corners", () => {
    const d = dashes([
      [0, 0],
      [10, 0],
      [10, 20],
    ]);
    // 0–3, 12–15 (corner at 10 → (10,2)–(10,5)), 24–27 → (10,14)–(10,17).
    expect(d.map((s) => [s.a, s.b])).toEqual([
      [
        [0, 0],
        [3, 0],
      ],
      [
        [10, 2],
        [10, 5],
      ],
      [
        [10, 14],
        [10, 17],
      ],
    ]);
  });

  it("rounds corners with tangent arcs, and lanes follow them concentrically", () => {
    const bend: Pt[] = [
      [0, 0],
      [20, 0],
      [20, 20],
    ];
    const c = fillet(bend, 10);
    // A left turn of 90° with radius 10: the arc runs from (10, 0) to (20, 10) about (10, 10).
    expect(close(c[1], [10, 0])).toBe(true);
    expect(close(c[c.length - 2], [20, 10])).toBe(true);
    for (const q of c.slice(1, -1)) expect(dist(q, [10, 10])).toBeCloseTo(10, 9);
    // The inside edge 3 m left of the centreline lies on radius 7 (to within the chords' sag).
    const g = road(bend, { lanes: [1, 1], laneWidth: 3, shoulder: 0, centre: "none", radius: 10 });
    const inner = g.segments.filter((s) => dist(s.a, [10, 10]) < 8);
    expect(inner.length).toBeGreaterThan(40);
    for (const s of inner) expect(dist(s.a, [10, 10])).toBeCloseTo(7, 3);
  });

  it("uses a configured dash pattern", () => {
    const g = road(
      [
        [0, 0],
        [20, 0],
      ],
      { lanes: [0, 0], laneWidth: 3.5, shoulder: 0, centre: "dashed", dash: [1, 4] },
    );
    // Painted 0–1, 5–6, 10–11, 15–16, then the edge lines of zero-lane sides (y = 0, twice).
    const dashesOnly = g.segments.filter((s) => s.dashed);
    expect(dashesOnly.map((s) => [s.a[0], s.b[0]])).toEqual([
      [0, 1],
      [5, 6],
      [10, 11],
      [15, 16],
    ]);
  });

  it("offsets a bend with mitred corners", () => {
    const g = road(
      [
        [0, 0],
        [10, 0],
        [10, 10],
      ],
      { lanes: [1, 1], laneWidth: 3, shoulder: 0, centre: "solid" },
    );
    // The left edge (inside of the bend) turns at (7, 3).
    const leftEdge = g.segments.filter((s) => close(s.b, [7, 3]) || close(s.a, [7, 3]));
    expect(leftEdge).toHaveLength(2);
  });
});
