// Putting an animation's movers in the 3D view at one moment: linked scene models posed,
// others as markers. Playback and renders both use it.
import * as THREE from "three";
import { vehicleSpec } from "../scene3d/library";
import type { SceneObject } from "../scene3d/model";
import type { Engine } from "../viewer3d/engine";
import { poseMatrix, type Animation, type Sample } from "./model";

type Model = Extract<SceneObject, { kind: "model" }>;

/** Rear axle's x in a vehicle model's frame (the model's origin is the body's centre). */
export function rearAxle(o: Model | undefined): number {
  if (o?.asset.type !== "vehicle") return 0;
  const a = o.asset;
  return a.length / 2 - vehicleSpec(a).frontOverhang - a.wheelbase;
}

/** Show each mover at its state; `view` is the view looked through, if any: from a driver's
 * seat the driver's own vehicle isn't drawn (its interior isn't modelled). */
export function showAt(
  e: Engine,
  a: Animation,
  models: Model[],
  states: [string, Sample][],
  view: string,
) {
  const o = e.origin;
  const poses = new Map<string, number[]>();
  const marks = new THREE.Group();
  for (const [id, s] of states) {
    const m = a.movers.find((x) => x.id === id);
    const obj = models.find((x) => x.id === m?.object);
    if (obj) {
      const mat = poseMatrix(s, rearAxle(obj));
      mat[12] -= o[0];
      mat[13] -= o[1];
      mat[14] -= o[2];
      poses.set(obj.id, mat);
    } else {
      const b = new THREE.Mesh(
        new THREE.SphereGeometry(0.25, 16, 12),
        new THREE.MeshBasicMaterial({ color: s.assumed ? 0xf0a030 : 0x30c0f0 }),
      );
      b.position.set(s.position[0] - o[0], s.position[1] - o[1], s.position[2] - o[2] + 0.25);
      marks.add(b);
    }
  }
  const seat = a.views.find((v) => v.id === view)?.kind;
  const own = seat?.kind === "driver" ? a.movers.find((m) => m.id === seat.mover)?.object : null;
  e.setPoses(poses, new Set(own ? [own] : []));
  e.setAnalysisOverlay("animation-now", marks.children.length ? marks : null);
}
