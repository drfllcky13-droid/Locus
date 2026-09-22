import * as THREE from "three";

// Project frame: right-handed, Z up, meters.
export function createScene() {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x1e1f22);

  const grid = new THREE.GridHelper(20, 20, 0x5a5d63, 0x3a3c40); // 1 m cells
  grid.rotation.x = Math.PI / 2; // GridHelper lies in XZ; lay it in XY
  scene.add(grid, new THREE.AxesHelper(1));

  const camera = new THREE.PerspectiveCamera(50, 1, 0.01, 10_000);
  camera.up.set(0, 0, 1);
  camera.position.set(8, -8, 6);
  camera.lookAt(0, 0, 0);

  return { scene, camera, grid };
}
