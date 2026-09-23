// Placing an image under the drawing: image pixels (u right, v down) to project metres by a
// similarity (uniform scale, rotation, translation). Pure.
import type { Pt } from "./model";

export interface Placement {
  /** Project position of the image's top-left corner (pixel (0, 0)). */
  origin: Pt;
  /** Metres per pixel. */
  pixel: number;
  /** Direction of the image's +u axis in the project frame (radians, anticlockwise from +x). */
  rotation: number;
}

export interface Pair {
  pixel: Pt;
  world: Pt;
}

export interface Calibration {
  placement: Placement;
  /** Distance from each pair's known position to where the calibrated image puts it (m). */
  residuals: number[];
  /** Root mean square of the residuals (m); 0 with two pairs (an exact fit, no check). */
  rms: number;
}

export function imageToWorld(pl: Placement, [u, v]: Pt): Pt {
  const [c, s] = [Math.cos(pl.rotation), Math.sin(pl.rotation)];
  // +u along (c, s); +v (down the image) along (s, −c), i.e. image-up is (−s, c).
  return [pl.origin[0] + pl.pixel * (u * c + v * s), pl.origin[1] + pl.pixel * (u * s - v * c)];
}

export function worldToImage(pl: Placement, [x, y]: Pt): Pt {
  const [c, s] = [Math.cos(pl.rotation), Math.sin(pl.rotation)];
  const [dx, dy] = [(x - pl.origin[0]) / pl.pixel, (y - pl.origin[1]) / pl.pixel];
  return [dx * c + dy * s, dx * s - dy * c];
}

/**
 * Least-squares similarity from two or more image points with known project positions. In
 * complex numbers w = a·z + b with z = u − i·v (image v runs down), which is linear in a and
 * b: a = Σ(w − w̄)·conj(z − z̄) / Σ|z − z̄|², b = w̄ − a·z̄. Two pairs fit exactly.
 */
export function calibrate(pairs: Pair[]): Calibration {
  if (pairs.length < 2) throw new Error("Calibration needs at least two points.");
  const n = pairs.length;
  const z = pairs.map((p) => [p.pixel[0], -p.pixel[1]] as Pt);
  const w = pairs.map((p) => p.world);
  const mean = (a: Pt[]): Pt => [
    a.reduce((s, p) => s + p[0], 0) / n,
    a.reduce((s, p) => s + p[1], 0) / n,
  ];
  const [zc, wc] = [mean(z), mean(w)];
  let [re, im, den] = [0, 0, 0];
  for (let i = 0; i < n; i++) {
    const [zx, zy] = [z[i][0] - zc[0], z[i][1] - zc[1]];
    const [wx, wy] = [w[i][0] - wc[0], w[i][1] - wc[1]];
    // (wx + i·wy)·(zx − i·zy)
    re += wx * zx + wy * zy;
    im += wy * zx - wx * zy;
    den += zx * zx + zy * zy;
  }
  if (den === 0) throw new Error("The calibration points are all at the same place in the image.");
  const [ar, ai] = [re / den, im / den];
  const origin: Pt = [wc[0] - (ar * zc[0] - ai * zc[1]), wc[1] - (ar * zc[1] + ai * zc[0])];
  const placement = { origin, pixel: Math.hypot(ar, ai), rotation: Math.atan2(ai, ar) };
  if (!(placement.pixel > 0)) throw new Error("The known points are all at the same place.");
  const residuals = pairs.map((p) => {
    const q = imageToWorld(placement, p.pixel);
    return Math.hypot(q[0] - p.world[0], q[1] - p.world[1]);
  });
  const rms = Math.sqrt(residuals.reduce((s, r) => s + r * r, 0) / n);
  return { placement, residuals, rms };
}
