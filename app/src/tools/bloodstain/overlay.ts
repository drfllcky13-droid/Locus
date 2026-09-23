// Bloodstain area of origin in the 3D view: each stain's path back to the point nearest the
// origin (used: dark red; left out: grey, dashed), the origin and its 95 % ellipsoid, the floor
// stains' plan-view convergence when asked for (blue, on the floor), and the photo being worked
// on laid on its surface (with its opacity), to check the alignment.
// Relative to the view's render origin.
import * as THREE from "three";
import type { Alignment, BloodstainRun } from "../../api";

type V3 = [number, number, number];

const USED = 0xc0392b;
const UNUSED = 0x9a9a9a;
const REGION = 0xff6b5a;
const FLOOR = 0x3f8fe0;

export interface PhotoLayer {
  url: string;
  width: number;
  height: number;
  alignment: Alignment;
  opacity: number;
}

export function bloodstainOverlay(
  run: BloodstainRun | null,
  photo: PhotoLayer | null,
  origin: V3,
  onLoad: () => void,
): THREE.Group {
  const g = new THREE.Group();
  g.name = "bloodstain";
  const rel = (p: V3) => new THREE.Vector3(p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]);
  if (run) {
    const o = run.origin.point;
    run.stains.forEach((s, k) => {
      const c = run.inputs[k].centre;
      const d = [0, 1, 2].map((i) => o[i] - c[i]);
      const along = Math.max(0.05, d[0] * s.ray[0] + d[1] * s.ray[1] + d[2] * s.ray[2]);
      const end: V3 = [0, 1, 2].map((i) => c[i] + s.ray[i] * along) as V3;
      const geo = new THREE.BufferGeometry().setFromPoints([rel(c), rel(end)]);
      const mat = s.used
        ? new THREE.LineBasicMaterial({ color: USED, depthTest: false })
        : new THREE.LineDashedMaterial({
            color: UNUSED,
            dashSize: 0.05,
            gapSize: 0.04,
            depthTest: false,
          });
      const line = new THREE.Line(geo, mat);
      if (!s.used) line.computeLineDistances();
      line.renderOrder = 10;
      g.add(line);
    });
    const dot = new THREE.Mesh(
      new THREE.SphereGeometry(0.015, 16, 12),
      new THREE.MeshBasicMaterial({ color: USED, depthTest: false }),
    );
    dot.position.copy(rel(o));
    dot.renderOrder = 11;
    g.add(dot);
    // The 95 % ellipsoid: a unit sphere scaled by the half-axes, turned onto the axes.
    const e = run.origin.ellipsoid;
    const ell = new THREE.Mesh(
      new THREE.SphereGeometry(1, 24, 16),
      new THREE.MeshBasicMaterial({
        color: REGION,
        wireframe: true,
        transparent: true,
        opacity: 0.5,
        depthTest: false,
      }),
    );
    const [a, b, c] = e.axes.map((v) => new THREE.Vector3(...v));
    ell.matrixAutoUpdate = false;
    ell.matrix.makeBasis(
      a.multiplyScalar(e.semi_axes[0]),
      b.multiplyScalar(e.semi_axes[1]),
      c.multiplyScalar(e.semi_axes[2]),
    );
    ell.matrix.setPosition(rel(o));
    ell.renderOrder = 11;
    g.add(ell);
    const cv = run.convergence;
    if (cv) {
      const z = run.parameters.floor_z + 0.002;
      const at = (x: number, y: number) => rel([x, y, z]);
      const mat = new THREE.LineBasicMaterial({ color: FLOOR, depthTest: false });
      for (const k of cv.stains) {
        const c = run.inputs[k].centre;
        const line = new THREE.Line(
          new THREE.BufferGeometry().setFromPoints([at(c[0], c[1]), at(cv.point[0], cv.point[1])]),
          mat,
        );
        line.renderOrder = 10;
        g.add(line);
      }
      const ring: THREE.Vector3[] = [];
      for (let k = 0; k <= 48; k++) {
        const t = (k / 48) * Math.PI * 2;
        const [u, v] = [cv.semi_axes[0] * Math.cos(t), cv.semi_axes[1] * Math.sin(t)];
        ring.push(
          at(
            cv.point[0] + u * cv.axis[0] - v * cv.axis[1],
            cv.point[1] + u * cv.axis[1] + v * cv.axis[0],
          ),
        );
      }
      const loop = new THREE.Line(new THREE.BufferGeometry().setFromPoints(ring), mat);
      loop.renderOrder = 10;
      g.add(loop);
    }
  }
  if (photo) {
    const geo = photoGeometry(photo.alignment, photo.width, photo.height, rel);
    const tex = new THREE.TextureLoader().load(photo.url, onLoad);
    tex.colorSpace = THREE.SRGBColorSpace;
    const quad = new THREE.Mesh(
      geo,
      new THREE.MeshBasicMaterial({
        map: tex,
        transparent: true,
        opacity: photo.opacity,
        side: THREE.DoubleSide,
        // Written, so the eye-dome pass keeps the photo where no scan points lie behind it.
        depthWrite: true,
      }),
    );
    quad.renderOrder = 9;
    g.add(quad);
  }
  return g;
}

/** Where a photo pixel lies on its surface (render-relative), or null if the perspective
 * correction sends it past the horizon or more than 1.5 m from the stain. */
export function photoPoint(al: Alignment, x: number, y: number): V3 | null {
  const h = al.rectification?.h;
  let q: [number, number] = [x, y];
  if (h) {
    const w = h[2][0] * x + h[2][1] * y + h[2][2];
    const [cx, cy] = [0, 1].map(
      (k) => al.rectification!.corners_px.reduce((s, c) => s + c[k], 0) / 4,
    );
    const wc = h[2][0] * cx + h[2][1] * cy + h[2][2];
    if (!(w * wc > 0)) return null;
    q = [(h[0][0] * x + h[0][1] * y + h[0][2]) / w, (h[1][0] * x + h[1][1] * y + h[1][2]) / w];
  }
  const p = [0, 1, 2].map((i) => al.origin[i] + q[0] * al.x_step[i] + q[1] * al.y_step[i]) as V3;
  const d = Math.hypot(...[0, 1, 2].map((i) => p[i] - al.plane_point[i]));
  return d <= 1.5 ? p : null;
}

/** The photo as a mesh on its surface: one quad for a square-on photo, a 24 × 24 grid when
 * corrected for perspective (the correction isn't affine, so a single quad would bend it).
 * Lifted off the surface toward the viewer by three times the surface's roughness (at least
 * 1 mm), so the scan points don't show through it. */
function photoGeometry(
  al: Alignment,
  width: number,
  height: number,
  rel: (p: V3) => THREE.Vector3,
): THREE.BufferGeometry {
  const n = al.rectification ? 24 : 1;
  const lift = new THREE.Vector3(...al.plane_normal).multiplyScalar(
    Math.max(0.001, 3 * (al.plane_rms ?? 0)),
  );
  const pos: number[] = [];
  const uv: number[] = [];
  const ok: boolean[] = [];
  for (let j = 0; j <= n; j++)
    for (let i = 0; i <= n; i++) {
      const p = photoPoint(al, (i / n) * width, (j / n) * height);
      const v = p ? rel(p).add(lift) : new THREE.Vector3();
      ok.push(p !== null);
      pos.push(v.x, v.y, v.z);
      uv.push(i / n, 1 - j / n);
    }
  const index: number[] = [];
  for (let j = 0; j < n; j++)
    for (let i = 0; i < n; i++) {
      const a = j * (n + 1) + i;
      const [b, c, d] = [a + 1, a + n + 2, a + n + 1];
      if (ok[a] && ok[b] && ok[c] && ok[d]) index.push(a, b, c, a, c, d);
    }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
  geo.setAttribute("uv", new THREE.Float32BufferAttribute(uv, 2));
  geo.setIndex(index);
  return geo;
}
