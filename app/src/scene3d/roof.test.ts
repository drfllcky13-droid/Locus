import { describe, expect, it } from "vitest";
import type { Pt } from "../diagram2d/model";
import { rectangle, roof, type RoofParams } from "./roof";

// An 8 × 5 m building turned 30°, far from the origin.
const ang = Math.PI / 6;
const [c, s] = [Math.cos(ang), Math.sin(ang)];
const O: Pt = [431_200, 5_390_110];
const rect: Pt[] = [
  [-4, -2.5],
  [4, -2.5],
  [4, 2.5],
  [-4, 2.5],
].map(([x, y]) => [O[0] + x * c - y * s, O[1] + x * s + y * c]);
const P: RoofParams = { type: "gable", eaves: 212.6, pitch: 30, overhang: 0.4, thickness: 0.2 };
const k = Math.tan(Math.PI / 6);

const zs = (m: { positions: number[] }) => m.positions.filter((_, i) => i % 3 === 2);
/** Horizontal distance of each vertex from the building's long axis. */
const across = (m: { positions: number[] }) => {
  const out: number[] = [];
  for (let i = 0; i < m.positions.length; i += 3) {
    const [x, y] = [m.positions[i] - O[0], m.positions[i + 1] - O[1]];
    out.push(Math.abs(-x * s + y * c));
  }
  return out;
};

describe("roofs", () => {
  it("recognises a rectangle and its long axis", () => {
    const r = rectangle(rect)!;
    expect(r.length).toBeCloseTo(8, 9);
    expect(r.width).toBeCloseTo(5, 9);
    expect(Math.abs(r.u[0] * c + r.u[1] * s)).toBeCloseTo(1, 12);
    expect(rectangle([...rect.slice(0, 3), [rect[3][0] + 0.3, rect[3][1]]])).toBeNull();
  });

  it("puts a gable's ridge at eaves + half the width × tan(pitch), sitting on the wall line", () => {
    const m = roof(rect, P);
    const z = zs(m);
    expect(Math.max(...z)).toBeCloseTo(P.eaves + 2.5 * k, 9);
    // Every vertex's height matches the slope through the wall line at the eaves.
    const d = across(m);
    for (let i = 0; i < z.length; i++) {
      const onSlope = P.eaves + (2.5 - d[i]) * k;
      // Gable end walls start at the eaves on the wall line; everything else is on a slope.
      expect(Math.abs(z[i] - onSlope) < 1e-9 || Math.abs(z[i] - P.eaves) < 1e-9).toBe(true);
    }
    expect(Math.min(...z)).toBeCloseTo(P.eaves - 0.4 * k, 9);
  });

  it("builds a hip roof with a ridge shortened by the width", () => {
    const m = roof(rect, { ...P, type: "hip" });
    const top = Math.max(...zs(m));
    expect(top).toBeCloseTo(P.eaves + 2.5 * k, 9);
    // Ridge ends: the vertices at the top are 1.5 m either side of the centre.
    const ends: number[] = [];
    for (let i = 0; i < m.positions.length; i += 3)
      if (Math.abs(m.positions[i + 2] - top) < 1e-9)
        ends.push(Math.hypot(m.positions[i] - O[0], m.positions[i + 1] - O[1]));
    expect(Math.max(...ends)).toBeCloseTo(1.5, 9);
  });

  it("slopes a shed roof across the width", () => {
    const z = zs(roof(rect, { ...P, type: "shed" }));
    expect(Math.max(...z)).toBeCloseTo(P.eaves + (5 + 0.4) * k, 9);
    expect(Math.min(...z)).toBeCloseTo(P.eaves - 0.4 * k, 9);
  });

  it("lays a flat roof on any outline, grown by the overhang", () => {
    const L: Pt[] = [
      [0, 0],
      [6, 0],
      [6, 3],
      [3, 3],
      [3, 6],
      [0, 6],
    ];
    const m = roof(L, { ...P, type: "flat" });
    const xs = m.positions.filter((_, i) => i % 3 === 0);
    expect(Math.min(...xs)).toBeCloseTo(-0.4, 9);
    expect(Math.max(...xs)).toBeCloseTo(6.4, 9);
    expect(Math.max(...zs(m))).toBeCloseTo(P.eaves + 0.2, 12);
    expect(() => roof(L, P)).toThrow(/rectangular/);
  });
});
