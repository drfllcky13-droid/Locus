import { describe, expect, it } from "vitest";
import { direction, faces, lookup } from "./panorama";

describe("360° views", () => {
  const north: [number, number, number] = [0, 1, 0];
  const close = (a: number[], b: number[]) => a.forEach((x, i) => expect(x).toBeCloseTo(b[i], 2));

  it("centres the panorama on forward, right a quarter across, up at the top", () => {
    const [w, h] = [400, 200];
    close(direction(north, 199.5, 99.5, w, h), [0, 1, 0]);
    // Facing north, right is east (+x).
    close(direction(north, 299.5, 99.5, w, h), [1, 0, 0]);
    close(direction(north, 99.5, 99.5, w, h), [-1, 0, 0]);
    expect(direction(north, 10, 0, w, h)[2]).toBeGreaterThan(0.99);
  });

  it("finds each direction in the right cube face at the right pixel", () => {
    const fs = faces(north);
    // Straight ahead: the forward face's centre.
    expect(lookup(fs, [0, 1, 0], 100)).toEqual([0, 50, 50]);
    // 45° right of ahead, level: the forward face's right edge, half way down.
    const [k, x, y] = lookup(fs, [Math.SQRT1_2 + 1e-9, Math.SQRT1_2, 0], 100);
    expect([k, Math.round(x), Math.round(y)]).toEqual([2, 0, 50]);
    // Up and a little ahead: the top face, below its centre (toward forward).
    const [k2, , y2] = lookup(fs, [0, 0.2, 1], 100);
    expect(k2).toBe(4);
    expect(y2).toBeGreaterThan(50);
    // Down: the bottom face.
    expect(lookup(fs, [0, 0, -1], 100)[0]).toBe(5);
  });
});
