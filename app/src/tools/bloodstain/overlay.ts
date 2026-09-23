// Bloodstain area of origin in the 3D view: each stain's path back to the point nearest the
// origin (used: dark red; left out: grey, dashed), the origin and its 95 % ellipsoid, and the
// photo being worked on laid on its surface (with its opacity), to check the alignment.
// Relative to the view's render origin.
import * as THREE from "three";
import type { Alignment, BloodstainRun } from "../../api";

type V3 = [number, number, number];

const USED = 0xc0392b;
const UNUSED = 0x9a9a9a;
const REGION = 0xff6b5a;

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
  }
  if (photo) {
    const al = photo.alignment;
    const at = (x: number, y: number) =>
      rel([0, 1, 2].map((i) => al.origin[i] + x * al.x_step[i] + y * al.y_step[i]) as V3)
        // A millimetre off the surface, toward the viewer's side, so the points don't hide it.
        .addScaledVector(new THREE.Vector3(...al.plane_normal), 0.001);
    const [w, h] = [photo.width, photo.height];
    const geo = new THREE.BufferGeometry().setFromPoints([at(0, 0), at(w, 0), at(w, h), at(0, h)]);
    geo.setIndex([0, 1, 2, 0, 2, 3]);
    geo.setAttribute("uv", new THREE.Float32BufferAttribute([0, 1, 1, 1, 1, 0, 0, 0], 2));
    const tex = new THREE.TextureLoader().load(photo.url, onLoad);
    tex.colorSpace = THREE.SRGBColorSpace;
    const quad = new THREE.Mesh(
      geo,
      new THREE.MeshBasicMaterial({
        map: tex,
        transparent: true,
        opacity: photo.opacity,
        side: THREE.DoubleSide,
        depthWrite: false,
      }),
    );
    quad.renderOrder = 9;
    g.add(quad);
  }
  return g;
}
