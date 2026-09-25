// The 3D scene as glTF binary (.glb): what the scene builder builds (extrusions, roofs, models,
// imported meshes, lights), not the point cloud. glTF is y-up; the project frame is z-up, so
// the scene is turned −90° about x, and the root node's extras say so, with the offset the
// geometry is relative to.
import * as THREE from "three";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import type { SceneDoc } from "./model";
import { buildScene, type Diagrams, type Meshes } from "./render";

export async function sceneGlb(
  doc: SceneDoc,
  diagrams: Diagrams,
  origin: [number, number, number],
  meshes: Meshes,
): Promise<ArrayBuffer> {
  const built = buildScene(doc, diagrams, origin, meshes);
  const root = new THREE.Group();
  root.name = "Lotus scene";
  root.rotation.x = -Math.PI / 2;
  root.userData = {
    locus: {
      units: "metres",
      frame: "project frame (x east, y north, z up), turned to glTF's y up",
      // Add this to a vertex (after turning back to z up) for project coordinates.
      origin,
    },
  };
  root.add(built);
  root.updateMatrixWorld(true);
  const out = await new GLTFExporter().parseAsync(root, { binary: true });
  return out as ArrayBuffer;
}
