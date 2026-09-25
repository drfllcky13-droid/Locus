// Imported meshes (OBJ, glTF/GLB, PLY) as three.js objects in project metres, z up. The bytes
// come from the backend only after their SHA-256 matches the one recorded at import.
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { OBJLoader } from "three/examples/jsm/loaders/OBJLoader.js";
import { PLYLoader } from "three/examples/jsm/loaders/PLYLoader.js";
import { api, type EvidenceRecord } from "../api";
import { UNITS } from "../format";
import { fileToProject } from "./model";

export const isMesh = (e: EvidenceRecord) => e.contents.meshes.length > 0;

/** Bottom centre of a loaded mesh's bounds, in project metres (its placement pivot). */
export function basePoint(m: THREE.Object3D): [number, number, number] {
  const b = new THREE.Box3().setFromObject(m);
  const c = b.getCenter(new THREE.Vector3());
  return [c.x, c.y, b.min.z];
}

/** The size of an object's bounds along x, y, z. */
export function extentOf(m: THREE.Object3D): [number, number, number] {
  const s = new THREE.Box3().setFromObject(m).getSize(new THREE.Vector3());
  return [s.x, s.y, s.z];
}

const grey = () =>
  new THREE.MeshStandardMaterial({ color: 0xb0b0b0, roughness: 0.8, side: THREE.DoubleSide });

async function parse(e: EvidenceRecord, bytes: ArrayBuffer): Promise<THREE.Object3D> {
  switch (e.contents.format) {
    case "glTF":
    case "GLB":
      // External buffers and images are not part of the evidence item; the loader can't reach
      // them, and says so.
      return (await new GLTFLoader().parseAsync(bytes, "")).scene;
    case "OBJ": {
      // Materials live in a separate .mtl that is not part of the evidence item.
      const g = new OBJLoader().parse(new TextDecoder().decode(bytes));
      g.traverse((o) => {
        if (o instanceof THREE.Mesh) o.material = grey();
      });
      return g;
    }
    case "PLY": {
      const geo = new PLYLoader().parse(bytes);
      if (!geo.hasAttribute("normal")) geo.computeVertexNormals();
      const m = grey();
      if (geo.hasAttribute("color")) {
        m.vertexColors = true;
        m.color.set(0xffffff);
      }
      return new THREE.Mesh(geo, m);
    }
    default:
      throw new Error(`${e.contents.format} is not a mesh format.`);
  }
}

// ponytail: loaded meshes stay in memory for the session; evict if large meshes pile up.
const cache = new Map<string, Promise<THREE.Object3D>>();

/**
 * The mesh of evidence item `e`, converted to project metres, z up. Share it with `clone()`:
 * its geometry and materials are marked `keep`, so the view never disposes the cached copy.
 */
export function loadEvidenceMesh(e: EvidenceRecord, root: string): Promise<THREE.Object3D> {
  // Per project: another project's copy of the same file is checked again.
  const key = `${root}|${e.sha256}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const metres = UNITS.find((u) => u.value === e.unit)?.metres;
  if (!metres)
    return Promise.reject(
      new Error(`Evidence #${e.id} has no unit recorded, so it can't be drawn to scale.`),
    );
  const p = api.meshBytes(e.id).then(async (bytes) => {
    const obj = await parse(e, bytes);
    obj.traverse((o) => {
      o.userData.keep = true;
      o.castShadow = o.receiveShadow = true;
    });
    const group = new THREE.Group();
    group.name = `Evidence #${e.id}`;
    group.matrixAutoUpdate = false;
    group.matrix.fromArray(fileToProject(metres, e.contents.y_up));
    group.userData.keep = true;
    group.add(obj);
    group.updateMatrixWorld(true);
    return group;
  });
  cache.set(key, p);
  p.catch(() => cache.delete(key)); // a refused file is checked again next time
  return p;
}
