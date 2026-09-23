// A trajectory in the 3D view: the fitted path (traced back through the height band and a
// little past the last point), the edges of the cone from the first point, the band's
// footprint on the floor, and the points used. Relative to the view's render origin.
import * as THREE from "three";
import type { TrajectoryRun } from "../../api";

type V3 = [number, number, number];

const PATH = 0xf2a53a;
const CONE = 0xd98e1f;
const BAND = 0x6fb7ff;

export function trajectoryOverlay(run: TrajectoryRun, origin: V3): THREE.Group {
  const g = new THREE.Group();
  g.name = "trajectory";
  const rel = (p: V3) => new THREE.Vector3(p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]);
  const d = new THREE.Vector3(...run.line.direction);
  const first = run.inputs[0].point;
  const last = run.inputs[run.inputs.length - 1].point;
  const back = run.band.centre ? run.band.centre[0][1] : 3;
  const line = (pts: THREE.Vector3[], color: number, dashed = false) => {
    const geo = new THREE.BufferGeometry().setFromPoints(pts);
    const mat = dashed
      ? new THREE.LineDashedMaterial({ color, dashSize: 0.2, gapSize: 0.12, depthTest: false })
      : new THREE.LineBasicMaterial({ color, depthTest: false });
    const l = new THREE.Line(geo, mat);
    if (dashed) l.computeLineDistances();
    l.renderOrder = 10;
    g.add(l);
  };
  const a = rel(first);
  // The path: from the far end of the band (or 3 m back) to 1 m past the last point.
  line([a.clone().addScaledVector(d, -Math.max(back, 1)), rel(last).addScaledVector(d, 1)], PATH);
  // Cone edges, back from the first point.
  const c = (run.parameters.cone_deg * Math.PI) / 180;
  const t = Math.abs(d.z) < 0.9 ? new THREE.Vector3(0, 0, 1) : new THREE.Vector3(1, 0, 0);
  const u = new THREE.Vector3().crossVectors(d, t).normalize();
  const w = new THREE.Vector3().crossVectors(d, u);
  const len = Math.max(back, 2);
  for (let k = 0; k < 8; k++) {
    const ang = (k / 8) * Math.PI * 2;
    const side = u.clone().multiplyScalar(Math.cos(ang)).addScaledVector(w, Math.sin(ang));
    const dk = d.clone().multiplyScalar(Math.cos(c)).addScaledVector(side, Math.sin(c));
    line([a, a.clone().addScaledVector(dk, -len)], CONE, true);
  }
  // The band's footprint, drawn on the floor.
  const f = run.band.footprint;
  if (f.length >= 3) {
    const z = run.parameters.floor_z;
    line(
      [...f, f[0]].map(([x, y]) => rel([x, y, z])),
      BAND,
    );
  }
  // The points used.
  const dot = new THREE.SphereGeometry(0.012, 12, 8);
  for (const p of run.inputs) {
    const m = new THREE.Mesh(dot, new THREE.MeshBasicMaterial({ color: PATH, depthTest: false }));
    m.position.copy(rel(p.point));
    m.renderOrder = 11;
    g.add(m);
  }
  return g;
}
