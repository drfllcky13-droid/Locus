import { describe, expect, it } from "vitest";
import type { Alignment } from "../../api";
import { photoPoint } from "./overlay";

const base: Alignment = {
  pairs: [],
  plane_point: [0, 0, 0],
  plane_normal: [0, 0, 1],
  origin: [0, 0, 0],
  x_step: [0.001, 0, 0],
  y_step: [0, -0.001, 0],
  pixels_per_metre: 1000,
  residuals: [],
  rms: null,
  rotation_sigma_deg: 0,
  rectification: null,
  scale_ratio: null,
  plane_rms: 0,
};

describe("photo on its surface", () => {
  it("places a square-on photo's pixels by the similarity", () => {
    expect(photoPoint(base, 100, 50)).toEqual([0.1, -0.05, 0]);
  });

  it("applies the perspective correction first, and drops points past its horizon", () => {
    const al: Alignment = {
      ...base,
      rectification: {
        corners_px: [
          [0, 0],
          [10, 0],
          [10, 10],
          [0, 10],
        ],
        size: [0.01, 0.01],
        corner_sigma_px: 1,
        h: [
          [1, 0, 0],
          [0, 1, 0],
          [0.001, 0, 1],
        ],
        pixels_per_metre: 1000,
        stretch: 0,
      },
    };
    const p = photoPoint(al, 100, 50)!;
    expect(p[0]).toBeCloseTo(0.1 / 1.1, 12);
    expect(p[1]).toBeCloseTo(-0.05 / 1.1, 12);
    expect(photoPoint(al, -2000, 0)).toBeNull();
  });
});
