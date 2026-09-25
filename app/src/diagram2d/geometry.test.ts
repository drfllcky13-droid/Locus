import { describe, expect, it } from "vitest";
import {
  corners,
  foot,
  length,
  movable,
  pickAt,
  snap,
  translate,
  type SnapOptions,
} from "./geometry";
import { road, room } from "./builders";
import type { Entity, Pt } from "./model";

const line = (id: string, a: [number, number], b: [number, number]): Entity => ({
  id,
  layer: "base",
  kind: "line",
  a,
  b,
});

const opts = (o: Partial<SnapOptions> = {}): SnapOptions => ({
  tolerance: 0.2,
  grid: 0,
  from: null,
  endpoint: true,
  midpoint: true,
  perpendicular: true,
  ...o,
});

describe("snapping", () => {
  const ents = [line("a", [0, 0], [10, 0]), line("b", [10, 0], [10, 6])];

  it("prefers an endpoint, then a midpoint, within tolerance", () => {
    expect(snap([9.9, 0.1], ents, opts())).toEqual({ point: [10, 0], kind: "endpoint" });
    expect(snap([5.1, -0.1], ents, opts())).toEqual({ point: [5, 0], kind: "midpoint" });
    expect(snap([5.5, 0.1], ents, opts()).kind).toBe("none");
  });

  it("snaps to the foot of the perpendicular from the line's start", () => {
    // Drawing from (3, 4): the perpendicular onto segment a lands at (3, 0).
    expect(snap([3.1, 0.05], ents, opts({ from: [3, 4] }))).toEqual({
      point: [3, 0],
      kind: "perpendicular",
    });
    // Not off the end of a segment.
    expect(snap([-2, 0], ents, opts({ from: [-2, 4] })).kind).toBe("none");
  });

  it("falls back to the grid, and each snap can be turned off", () => {
    expect(snap([5.52, 0.1], ents, opts({ grid: 0.25 }))).toEqual({
      point: [5.5, 0],
      kind: "grid",
    });
    expect(snap([9.9, 0.1], ents, opts({ endpoint: false })).kind).toBe("none");
  });
});

describe("geometry", () => {
  it("perpendicular foot and lengths", () => {
    expect(foot([2, 3], [0, 0], [4, 0])).toEqual([2, 0]);
    expect(foot([1, 1], [0, 0], [0, 0])).toBeNull();
    expect(length(line("x", [0, 0], [3, 4]))).toBe(5);
    const arc: Entity = {
      id: "c",
      layer: "base",
      kind: "arc",
      center: [0, 0],
      radius: 2,
      start: 0,
      end: Math.PI,
    };
    expect(length(arc)).toBeCloseTo(2 * Math.PI, 12);
    const poly: Entity = {
      id: "p",
      layer: "base",
      kind: "polyline",
      points: [
        [0, 0],
        [3, 0],
        [3, 4],
      ],
      closed: true,
    };
    expect(length(poly)).toBe(12);
  });
});

describe("moving and finishing", () => {
  it("a double-clicked last corner would make a zero-length wall; corners() drops it", () => {
    const clicked: Pt[] = [
      [0, 0],
      [4, 0],
      [4, 3],
      [0, 3],
      [0, 3],
    ];
    // The repeated corner loses two outer wall faces (inner and outer face per wall: 8 lines).
    expect(room(clicked, 0.15, []).segments).toHaveLength(6);
    const pts = corners(clicked, true);
    expect(pts).toHaveLength(4);
    expect(room(pts, 0.15, []).segments).toHaveLength(8);
    // Closing on the first corner is the same room.
    expect(corners([...pts, [0, 0]], true)).toEqual(pts);
  });

  it("translate moves every kind by the offset, and built geometry with it", () => {
    const r = room(
      [
        [0, 0],
        [4, 0],
        [4, 3],
      ],
      0.15,
      [],
    );
    const e: Entity = {
      id: "r",
      layer: "base",
      kind: "room",
      outline: [
        [0, 0],
        [4, 0],
        [4, 3],
      ],
      thickness: 0.15,
      openings: [],
      geometry: r,
    };
    const m = translate(e, [1, 2]);
    if (m.kind !== "room") throw new Error();
    expect(m.outline[1]).toEqual([5, 2]);
    expect(m.geometry.segments[0].a).toEqual([r.segments[0].a[0] + 1, r.segments[0].a[1] + 2]);
    const s = translate(
      { id: "s", layer: "base", kind: "symbol", symbol: "car", at: [1, 1], rotation: 0, scale: 1 },
      [-1, 0.5],
    );
    expect(s.kind === "symbol" && s.at).toEqual([0, 1.5]);
  });

  it("measured points and underlays don't move", () => {
    const p: Entity = {
      id: "p",
      layer: "base",
      kind: "point",
      at: [0, 0],
      label: "A",
      measurement: { method: "triangulation", refs: [], side: null },
      sigma: 0.01,
    };
    expect(movable(p, [p])).toBe(false);
    expect(movable({ ...p, measurement: null }, [])).toBe(true);
  });

  it("reference points a measured point was taken from don't move", () => {
    type Point = Extract<Entity, { kind: "point" }>;
    const ref = (id: string): Point => ({
      id,
      layer: "base",
      kind: "point",
      at: [0, 0],
      label: id,
      measurement: null,
      sigma: null,
    });
    const tri: Entity = {
      ...ref("m"),
      measurement: { method: "triangulation", refs: [{ point: "a", distance: 5 }], side: null },
    };
    const base: Entity = {
      ...ref("n"),
      measurement: {
        method: "baseline_offset",
        from: "b",
        to: "c",
        along: 1,
        offset: 1,
        side: "Left",
      },
    };
    const all = [ref("a"), ref("b"), ref("c"), ref("d"), tri, base];
    expect(all.filter((e) => movable(e, all)).map((e) => e.id)).toEqual(["d"]);
  });
});

describe("picking", () => {
  const edge: Entity = { id: "edge", layer: "base", kind: "line", a: [-5, -2], b: [5, -2] };
  // A marker snapped onto the edge, a hair off where the user then clicks.
  const marker: Entity = {
    id: "m",
    layer: "base",
    kind: "marker",
    number: 2,
    at: [-0.02, -2],
    note: "",
  };

  it("an item placed at a point wins over a line it sits on", () => {
    expect(pickAt([edge, marker], [0, -2.01], 0.2)?.id).toBe("m");
    expect(pickAt([marker, edge], [0, -2.01], 0.2)?.id).toBe("m");
  });

  it("a road is picked on its centre line, painted or not", () => {
    const params = {
      lanes: [1, 1] as [number, number],
      laneWidth: 3.5,
      shoulder: 1,
      centre: "none" as const,
    };
    const centreline: Pt[] = [
      [-7.5, 0],
      [7.5, 0],
    ];
    const r: Entity = {
      id: "r",
      layer: "base",
      kind: "road",
      centreline,
      road: params,
      geometry: road(centreline, params),
    };
    expect(pickAt([r], [0, 0.1], 0.2)?.id).toBe("r");
  });

  it("otherwise the nearest within reach, and nothing beyond it", () => {
    const other: Entity = { id: "o", layer: "base", kind: "line", a: [-5, -2.1], b: [5, -2.1] };
    expect(pickAt([edge, other], [3, -2.08], 0.2)?.id).toBe("o");
    expect(pickAt([edge, marker], [0, -3], 0.2)).toBeUndefined();
  });
});
