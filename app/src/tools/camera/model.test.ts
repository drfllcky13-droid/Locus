import { describe, expect, it } from "vitest";
import type { SolvedCamera } from "../../api";
import { maxRadius, project } from "./model";

// Looking along +y from the origin: right is +x, down is −z.
const cam: SolvedCamera = {
  position: [0, 0, 0],
  rotation: [
    [1, 0, 0],
    [0, 0, -1],
    [0, 1, 0],
  ],
  size: [1000, 800],
  f: 800,
  cx: 500,
  cy: 400,
  distortion: [0, 0, 0, 0, 0],
};

describe("camera model", () => {
  it("projects the optical axis to the principal point, and up to up", () => {
    expect(project(cam, [0, 5, 0])).toEqual([500, 400]);
    const up = project(cam, [0, 5, 1])!;
    expect(up[1]).toBeLessThan(400);
    expect(project(cam, [0, -5, 0])).toBeNull();
  });

  it("applies radial distortion as the Rust model does", () => {
    const d: SolvedCamera = { ...cam, distortion: [-0.2, 0.05, 0, 0, 0] };
    // a = 0.5: 0.5 · (1 − 0.2·0.25 + 0.05·0.0625) = 0.4765625.
    const p = project(d, [0.5, 1, 0])!;
    expect(p[0]).toBeCloseTo(500 + 800 * 0.4765625, 9);
  });

  it("stops at the lens model's fold", () => {
    const d: SolvedCamera = { ...cam, distortion: [-0.28, 0.09, -0.012, 0, 0] };
    const r = maxRadius(d);
    expect(r).toBeGreaterThan(1.7);
    expect(r).toBeLessThan(2);
    expect(project(d, [2.5, 1, 0])).toBeNull();
  });
});
