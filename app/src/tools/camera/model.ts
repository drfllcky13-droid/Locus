// The solved camera's projection in the webview, the same model as locus-analysis camera.rs
// (OpenCV's): camera axes x right, y down, z forward; radial k1–k3 and tangential p1, p2
// distortion; square pixels. Used to draw scan points and person models over the photo.
import type { SolvedCamera } from "../../api";

type V3 = [number, number, number];
type P2 = [number, number];

export function distort(c: SolvedCamera, a: number, b: number): P2 {
  const [k1, k2, k3, p1, p2] = c.distortion;
  const r2 = a * a + b * b;
  const radial = 1 + k1 * r2 + k2 * r2 * r2 + k3 * r2 * r2 * r2;
  return [
    a * radial + 2 * p1 * a * b + p2 * (r2 + 2 * a * a),
    b * radial + p1 * (r2 + 2 * b * b) + 2 * p2 * a * b,
  ];
}

/** The largest undistorted radius the lens model is valid for (where the radial distortion
 * stops increasing); beyond it the polynomial folds back. */
export function maxRadius(c: SolvedCamera): number {
  const [k1, k2, k3] = c.distortion;
  for (let i = 1; i <= 20000; i++) {
    const r2 = i * 0.005;
    if (1 + 3 * k1 * r2 + 5 * k2 * r2 * r2 + 7 * k3 * r2 * r2 * r2 <= 0) return Math.sqrt(r2);
  }
  return Infinity;
}

/** Pixel of a project-frame point, or null behind the camera or past the lens's fold. */
export function project(c: SolvedCamera, x: V3, rmax = maxRadius(c)): P2 | null {
  const d = [x[0] - c.position[0], x[1] - c.position[1], x[2] - c.position[2]];
  const r = c.rotation;
  const z = r[2][0] * d[0] + r[2][1] * d[1] + r[2][2] * d[2];
  if (z <= 1e-9) return null;
  const a = (r[0][0] * d[0] + r[0][1] * d[1] + r[0][2] * d[2]) / z;
  const b = (r[1][0] * d[0] + r[1][1] * d[1] + r[1][2] * d[2]) / z;
  if (Math.hypot(a, b) >= rmax) return null;
  const [ad, bd] = distort(c, a, b);
  return [c.f * ad + c.cx, c.f * bd + c.cy];
}
