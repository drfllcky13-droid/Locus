// The 3D scene document as Three.js objects, relative to the view's render origin. Geometry
// comes from the pure builders (extrude, roof, library); this file only turns it into GPU
// objects. Positions are subtracted from the origin in f64 before becoming f32.
import * as THREE from "three";
import type { Diagram } from "../diagram2d/model";
import { walls } from "../diagram2d/builders";
import { extrude, relative, type Mesh, type Part } from "./extrude";
import { build, type Slot } from "./library";
import { roof } from "./roof";
import { DEFAULT_MATERIAL, type MaterialDef, type SceneDoc, type SceneObject } from "./model";

type V3 = [number, number, number];

function geometry(m: Mesh, origin: V3): THREE.BufferGeometry {
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(relative(m, origin), 3));
  g.setIndex(m.indices);
  // Flat shading: faces keep hard edges, which reads better for built geometry.
  const flat = g.toNonIndexed();
  flat.computeVertexNormals();
  g.dispose();
  return flat;
}

function material(d: MaterialDef): THREE.MeshStandardMaterial {
  return new THREE.MeshStandardMaterial({
    color: new THREE.Color(d.color),
    roughness: d.roughness,
    metalness: d.metalness,
    transparent: d.opacity < 1,
    opacity: d.opacity,
    // Built geometry is closed but winding isn't guaranteed across every builder; draw both
    // sides so nothing vanishes from inside a room.
    side: THREE.DoubleSide,
  });
}

function meshObject(m: Mesh, origin: V3, mat: MaterialDef, id: string): THREE.Mesh {
  const o = new THREE.Mesh(geometry(m, origin), material(mat));
  o.castShadow = o.receiveShadow = true;
  o.userData.sceneObject = id;
  return o;
}

/** A label drawn on a canvas, as a texture (evidence marker numbers). */
function label(text: string): THREE.CanvasTexture {
  const c = document.createElement("canvas");
  c.width = c.height = 128;
  const g = c.getContext("2d")!;
  g.fillStyle = "#f2c94c";
  g.fillRect(0, 0, 128, 128);
  g.fillStyle = "#111";
  g.font = "bold 84px sans-serif";
  g.textAlign = "center";
  g.textBaseline = "middle";
  g.fillText(text, 64, 70);
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  return t;
}

/** Diagram revisions by revision id, loaded by the caller. */
export type Diagrams = Map<number, Diagram>;

function objectGroup(
  o: SceneObject,
  diagrams: Diagrams,
  origin: V3,
  all: SceneObject[],
): THREE.Object3D | null {
  const g = new THREE.Group();
  g.userData.sceneObject = o.id;
  switch (o.kind) {
    case "extrusion": {
      const d = diagrams.get(o.diagram.revision);
      if (!d) return null;
      const parts = extrude(d, o.params);
      for (const part of Object.keys(parts) as Part[])
        for (const m of parts[part])
          g.add(meshObject(m, origin, o.materials[part] ?? DEFAULT_MATERIAL[part], o.id));
      return g;
    }
    case "roof": {
      const d = diagrams.get(o.diagram.revision);
      const r = d?.entities.find((e) => e.id === o.room);
      if (!r || r.kind !== "room") return null;
      const outer = walls(r.outline, r.thickness, []).map((w) => w.oa);
      const ext = all.find((x) => x.id === o.extrusion);
      const base = ext?.kind === "extrusion" ? ext.params.base : 0;
      const params = { ...o.params, eaves: base + o.params.eaves };
      g.add(meshObject(roof(outer, params), origin, o.material, o.id));
      return g;
    }
    case "model": {
      // Model geometry is local and small; the placement carries the large coordinates, and
      // its translation is taken relative to the origin in f64.
      const model = build(o.asset);
      for (const slot of Object.keys(model) as Slot[])
        g.add(
          meshObject(model[slot]!, [0, 0, 0], o.materials[slot] ?? DEFAULT_MATERIAL[slot], o.id),
        );
      if (o.asset.type === "marker") {
        // The number on both sloping faces of the tent.
        const s = o.asset.size;
        const tex = label(String(o.asset.number));
        for (const side of [1, -1]) {
          const p = new THREE.Mesh(
            new THREE.PlaneGeometry(s * 0.45, s * 0.45),
            new THREE.MeshStandardMaterial({ map: tex, roughness: 0.6 }),
          );
          // Centred on the sloping face, facing out of it, text upright.
          const n = new THREE.Vector3(side * s, 0, s * 0.45).normalize();
          const c = new THREE.Vector3(side * s * 0.2, 0, s * 0.55).addScaledVector(n, 0.001);
          p.position.copy(c);
          p.up.set(0, 0, 1);
          p.lookAt(c.clone().add(n));
          p.userData.sceneObject = o.id;
          g.add(p);
        }
      }
      const m = new THREE.Matrix4().fromArray(o.matrix);
      m.elements[12] -= origin[0];
      m.elements[13] -= origin[1];
      m.elements[14] -= origin[2];
      g.matrixAutoUpdate = false;
      g.matrix.copy(m);
      return g;
    }
    case "light": {
      const l = o.light;
      const rel = (p: V3) =>
        new THREE.Vector3(p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]);
      if (l.type === "ambient") g.add(new THREE.AmbientLight(l.color, l.intensity));
      else if (l.type === "point") {
        const p = new THREE.PointLight(l.color, l.intensity, l.range, 2);
        p.position.copy(rel(l.position));
        p.castShadow = true;
        g.add(p);
      } else if (l.type === "spot") {
        const s = new THREE.SpotLight(l.color, l.intensity, 0, (l.angle * Math.PI) / 180, 0.2, 2);
        s.position.copy(rel(l.position));
        s.target.position.copy(rel(l.target));
        s.castShadow = true;
        g.add(s, s.target);
      } else {
        const dl = new THREE.DirectionalLight(l.color, l.intensity);
        dl.position.set(-l.direction[0], -l.direction[1], -l.direction[2]).multiplyScalar(100);
        g.add(dl, dl.target);
      }
      return g;
    }
  }
}

/**
 * Unit vector toward the sun in the project frame. `azimuth` is clockwise from true north;
 * true north is `north` degrees anticlockwise from project +y, so the bearing clockwise from
 * project +y is azimuth − north.
 */
export function sunDirection(azimuth: number, elevation: number, north: number): V3 {
  const b = ((azimuth - north) * Math.PI) / 180;
  const el = (elevation * Math.PI) / 180;
  return [Math.sin(b) * Math.cos(el), Math.cos(b) * Math.cos(el), Math.sin(el)];
}

/** Everything in the scene document, relative to `origin`; `extent` bounds the sun's shadows. */
export function buildScene(doc: SceneDoc, diagrams: Diagrams, origin: V3): THREE.Group {
  const root = new THREE.Group();
  root.name = "scene3d";
  for (const o of doc.objects) {
    if (!o.visible) continue;
    const g = objectGroup(o, diagrams, origin, doc.objects);
    if (g) root.add(g);
  }
  if (doc.ambient > 0) root.add(new THREE.HemisphereLight(0xffffff, 0x444444, doc.ambient));
  const sun = doc.sun;
  if (sun.on && sun.computed && sun.computed.apparent_elevation > -1) {
    const d = sunDirection(sun.computed.azimuth, sun.computed.apparent_elevation, sun.north);
    const light = new THREE.DirectionalLight(0xfff4e5, sun.intensity);
    // Aim at the middle of what's built, from far along the sun's direction.
    const box = new THREE.Box3().setFromObject(root);
    const c = box.isEmpty() ? new THREE.Vector3() : box.getCenter(new THREE.Vector3());
    const r = box.isEmpty() ? 50 : Math.max(10, box.getSize(new THREE.Vector3()).length());
    light.position.set(c.x + d[0] * r * 2, c.y + d[1] * r * 2, c.z + d[2] * r * 2);
    light.target.position.copy(c);
    light.castShadow = sun.shadows;
    const cam = light.shadow.camera;
    [cam.left, cam.right, cam.top, cam.bottom] = [-r, r, r, -r];
    cam.near = 0.1;
    cam.far = r * 5;
    light.shadow.mapSize.set(4096, 4096);
    light.shadow.bias = -0.0005;
    // Offset along the surface normal as well, or lit faces stripe with their own shadow.
    light.shadow.normalBias = 0.02;
    light.name = "sun";
    root.add(light, light.target);
  }
  return root;
}
