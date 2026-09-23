import { describe, expect, it } from "vitest";
import { foot, length, snap, type SnapOptions } from "./geometry";
import type { Entity } from "./model";

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
