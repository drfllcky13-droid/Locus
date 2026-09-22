import { expect, test } from "vitest";
import * as THREE from "three";
import { createScene } from "./scene";

test("scene uses the Z-up project frame", () => {
  const { camera, grid } = createScene();
  expect(camera.up.toArray()).toEqual([0, 0, 1]);

  grid.updateMatrixWorld();
  const pos = grid.geometry.getAttribute("position");
  const v = new THREE.Vector3();
  for (let i = 0; i < pos.count; i++) {
    v.fromBufferAttribute(pos, i).applyMatrix4(grid.matrixWorld);
    expect(Math.abs(v.z)).toBeLessThan(1e-9);
  }
});
