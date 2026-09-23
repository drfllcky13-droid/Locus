import { describe, expect, it } from "vitest";
import { calibrate, imageToWorld, worldToImage, type Placement } from "./underlay";
import type { Pt } from "./model";

const truth: Placement = { origin: [120.5, -40.25], pixel: 0.0625, rotation: 0.3 };

describe("underlay placement", () => {
  it("maps image down to project south when not rotated", () => {
    const pl: Placement = { origin: [0, 0], pixel: 0.5, rotation: 0 };
    expect(imageToWorld(pl, [2, 4])).toEqual([1, -2]);
  });

  it("round-trips", () => {
    const q = worldToImage(truth, imageToWorld(truth, [313, 97]));
    expect(q[0]).toBeCloseTo(313, 9);
    expect(q[1]).toBeCloseTo(97, 9);
  });
});

describe("calibrate", () => {
  it("recovers the placement exactly from two points", () => {
    const px: Pt[] = [
      [10, 20],
      [900, 640],
    ];
    const c = calibrate(px.map((p) => ({ pixel: p, world: imageToWorld(truth, p) })));
    expect(c.placement.pixel).toBeCloseTo(truth.pixel, 12);
    expect(c.placement.rotation).toBeCloseTo(truth.rotation, 12);
    expect(c.placement.origin[0]).toBeCloseTo(truth.origin[0], 9);
    expect(c.placement.origin[1]).toBeCloseTo(truth.origin[1], 9);
    expect(c.rms).toBeLessThan(1e-9);
  });

  it("reports the misfit of a wrong point", () => {
    const px: Pt[] = [
      [0, 0],
      [1000, 0],
      [1000, 1000],
      [0, 1000],
      [500, 500],
    ];
    const pairs = px.map((p) => ({ pixel: p, world: imageToWorld(truth, p) }));
    pairs[4].world = [pairs[4].world[0] + 0.3, pairs[4].world[1]];
    const c = calibrate(pairs);
    // The centre point's 0.3 m error barely moves the fit: it keeps 4/5 of it.
    expect(c.residuals[4]).toBeCloseTo(0.24, 9);
    expect(Math.max(...c.residuals)).toBe(c.residuals[4]);
  });

  it("refuses too few or coincident points", () => {
    expect(() => calibrate([{ pixel: [0, 0], world: [0, 0] }])).toThrow();
    expect(() =>
      calibrate([
        { pixel: [5, 5], world: [0, 0] },
        { pixel: [5, 5], world: [1, 1] },
      ]),
    ).toThrow();
    expect(() =>
      calibrate([
        { pixel: [0, 0], world: [1, 1] },
        { pixel: [5, 5], world: [1, 1] },
      ]),
    ).toThrow();
  });
});
