import { describe, expect, it } from "vitest";
import { sunDirection } from "./render";

describe("sun direction", () => {
  const close = (a: number[], b: number[]) => a.forEach((v, i) => expect(v).toBeCloseTo(b[i], 12));

  it("points along the bearing, up by the elevation", () => {
    close(sunDirection(0, 0, 0), [0, 1, 0]); // due north on the horizon: +y
    close(sunDirection(90, 0, 0), [1, 0, 0]); // due east: +x
    close(sunDirection(180, 90, 0), [0, 0, 1]); // overhead
    const d = sunDirection(135, 30, 0);
    expect(Math.hypot(...d)).toBeCloseTo(1, 12);
    expect(d[2]).toBeCloseTo(0.5, 12);
  });

  it("turns with the project's north", () => {
    // True north 90° anticlockwise from project +y, i.e. along −x.
    close(sunDirection(0, 0, 90), [-1, 0, 0]);
    close(sunDirection(90, 0, 90), [0, 1, 0]);
  });
});
