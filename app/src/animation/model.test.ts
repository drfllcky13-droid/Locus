import { describe, expect, it } from "vitest";
import type { CrashRecord } from "../api";
import { scaleBar } from "./overlay";
import { cameraAt, edrSegment, poseMatrix, sampleAt, type Sample } from "./model";

const s = (t: number, x: number, heading: number): Sample => ({
  t,
  position: [x, 0, 0],
  heading,
  distance: x,
  speed: 10,
  longitudinal: 0,
  lateral: 0,
  segment: 0,
  assumed: false,
});

describe("animation model", () => {
  it("interpolates between samples, heading the short way round", () => {
    const got = sampleAt([s(0, 0, Math.PI - 0.1), s(0.01, 0.1, -Math.PI + 0.1)], 0, 0.01, 0.005);
    expect(got.position[0]).toBeCloseTo(0.05, 12);
    expect(Math.abs(got.heading)).toBeCloseTo(Math.PI, 12);
  });

  it("puts a vehicle's body centre ahead of its rear axle", () => {
    // Heading +y; the rear axle 1.35 m behind the centre.
    const m = poseMatrix({ ...s(0, 2, Math.PI / 2) }, -1.35);
    expect(m[12]).toBeCloseTo(2, 12);
    expect(m[13]).toBeCloseTo(1.35, 12);
  });

  it("turns EDR stations into distance travelled from the first sample", () => {
    const spread = (value: number) => ({ value, low: value - 0.5, high: value + 1 });
    const r = {
      id: 7,
      name: "EDR 1",
      record: {
        path: [
          [0, 0, 0],
          [30, 0, 0],
        ],
        scale_tolerance: 0.01,
        offset_tolerance: 0.25,
        samples: [
          { t: -2, speed: 13 },
          { t: -1, speed: 11 },
          { t: 0, speed: 9 },
        ],
        stations: [
          { t: -2, distance: spread(25) },
          { t: -1, distance: spread(12) },
          { t: 0, distance: spread(2) },
        ],
      },
    } as unknown as CrashRecord;
    const got = edrSegment(r)!;
    expect(got.start).toBe(-2);
    expect(got.offset).toBeCloseTo(5, 12);
    expect(got.segment.duration).toBe(2);
    expect(got.segment.motion).toEqual({
      kind: "table",
      times: [0, 1, 2],
      distances: [0, 13, 23],
      speeds: [13, 11, 9],
      // Distance range: d0 - high .. d0 - low; speed: v(1 ± 1 %) ± 0.25 m/s.
      ranges: [
        [-1, 0.5],
        [12, 13.5],
        [22, 23.5],
      ],
      speed_ranges: [
        [12.62, 13.38],
        [10.64, 11.36],
        [8.66, 9.34],
      ],
    });
    expect(got.segment.source).toEqual({ kind: "edr", analysis_id: 7, name: "EDR 1" });
  });

  it("interpolates a view's camera between samples", () => {
    const c = cameraAt(
      [
        { eye: [0, 0, 1], target: [10, 0, 1] },
        { eye: [1, 0, 1], target: [10, 2, 1] },
      ],
      -1,
      0.01,
      -0.9975,
    );
    expect(c.eye[0]).toBeCloseTo(0.25, 12);
    expect(c.target[1]).toBeCloseTo(0.5, 12);
  });

  it("sizes the scale bar for its stated depth", () => {
    // 60° across 1920 × 1080 at 10 m: 166.3 px per metre, so 2 m fits in a quarter width.
    const s = scaleBar(1920, 1080, 60, 10);
    expect(s.metres).toBe(2);
    expect(s.px).toBeCloseTo((2 * 540 * 1920) / 1080 / (Math.tan(Math.PI / 6) * 10), 9);
  });
});
