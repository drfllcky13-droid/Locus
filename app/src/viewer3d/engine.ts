// The 3D view: camera, render-on-demand loop, point clouds, EDL, clipping, measurement
// overlays and picking. React only tells it what to show (see Viewport.tsx).
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { TransformControls } from "three/addons/controls/TransformControls.js";
import { CSS2DObject, CSS2DRenderer } from "three/addons/renderers/CSS2DRenderer.js";
import type { CleanupRequest, LassoDepth, Region, StateView } from "../api";
import { initialBudget, updateBudget, type BudgetState } from "./budget";
import { EdlPass } from "./edl";
import type { View } from "./lod";
import { formatMeasurement } from "./measureFormat";
import { PointCloudLayer, type ColorMode, type PickOutcome, type SceneData } from "./pointcloud";
import { createScene } from "./scene";

export interface Stats {
  fps: number;
  drawn: number;
  budget: number;
  selectionMs: number;
  loading: boolean;
}

export type ClipMode = "off" | "inside" | "outside";

/** Pixels around the cursor searched when picking, and the focus cone's width. */
export const SNAP_PX = 12;

export class Engine {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene: THREE.Scene;
  readonly camera: THREE.PerspectiveCamera;
  readonly controls: OrbitControls;
  private labels = new CSS2DRenderer();
  private edl = new EdlPass();
  private layer: PointCloudLayer | null = null;
  private overlays = new THREE.Group();
  private markers = new THREE.Group();
  private clipBox: THREE.Mesh;
  private grid: THREE.Object3D;
  private gizmo: TransformControls;
  /** Built 3D scene objects (scene3d/render.ts), and the gizmo that moves a placed model. */
  private built: THREE.Group | null = null;
  private modelGizmo: TransformControls;
  private moving: string | null = null;
  /** A placed model was moved with the gizmo: its new model-to-project matrix (f64). */
  onModelMoved: ((id: string, matrix: number[]) => void) | null = null;
  private dirty = true;
  private raf = 0;
  private lastFrame = 0;
  private budget: BudgetState = initialBudget();
  private fps = 0;
  private mouse: { x: number; y: number } | null = null;
  focusActive = false;
  clipMode: ClipMode = "off";
  private plane: { on: boolean; axis: 0 | 1 | 2; offset: number; flip: boolean } = {
    on: false,
    axis: 2,
    offset: 0,
    flip: false,
  };

  constructor(
    private host: HTMLElement,
    private onStats: (s: Stats) => void,
  ) {
    const { scene, camera, grid } = createScene();
    this.grid = grid;
    // The background is painted by the EDL pass (or the clear colour without it), not by
    // three.js, so it isn't colour-converted twice on the way through the sRGB target.
    scene.background = null;
    this.scene = scene;
    this.camera = camera;
    this.renderer = new THREE.WebGLRenderer({
      antialias: false,
      powerPreference: "high-performance",
    });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    this.renderer.setClearColor(0x1e1f22, 1);
    host.appendChild(this.renderer.domElement);
    this.labels.domElement.className = "labels";
    host.appendChild(this.labels.domElement);

    this.controls = new OrbitControls(camera, this.renderer.domElement);
    this.controls.addEventListener("change", () => this.requestRender());
    this.scene.add(this.overlays, this.markers);

    this.clipBox = new THREE.Mesh(
      new THREE.BoxGeometry(1, 1, 1),
      new THREE.MeshBasicMaterial({
        color: 0xf2a53a,
        wireframe: true,
        transparent: true,
        opacity: 0.6,
      }),
    );
    this.clipBox.visible = false;
    this.scene.add(this.clipBox);
    this.gizmo = new TransformControls(camera, this.renderer.domElement);
    this.gizmo.addEventListener("dragging-changed", (e) => (this.controls.enabled = !e.value));
    this.gizmo.addEventListener("objectChange", () => {
      this.updateClipUniforms();
      this.requestRender();
    });
    this.gizmo.addEventListener("change", () => this.requestRender());
    this.scene.add(this.gizmo.getHelper());
    this.modelGizmo = new TransformControls(camera, this.renderer.domElement);
    this.modelGizmo.addEventListener("dragging-changed", (e) => {
      this.controls.enabled = !e.value;
      // Report the move once, when the drag ends.
      const obj = this.modelGizmo.object;
      if (!e.value && obj && this.moving) {
        obj.updateMatrix();
        const m = obj.matrix.toArray();
        const o = this.origin;
        m[12] += o[0];
        m[13] += o[1];
        m[14] += o[2];
        this.onModelMoved?.(this.moving, m);
      }
    });
    this.modelGizmo.addEventListener("change", () => this.requestRender());
    this.scene.add(this.modelGizmo.getHelper());
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;

    this.renderer.domElement.addEventListener("pointermove", (e) => {
      const r = this.renderer.domElement.getBoundingClientRect();
      this.mouse = { x: e.clientX - r.left, y: e.clientY - r.top };
      if (this.focusActive) this.requestRender();
    });
    this.renderer.domElement.addEventListener("pointerleave", () => (this.mouse = null));
    // Handle for scripted measurements (performance runs drive `benchmark` over DevTools).
    (globalThis as Record<string, unknown>).__locus = this;
    new ResizeObserver(() => this.resize()).observe(host);
    this.resize();
    this.loop();
  }

  requestRender() {
    this.dirty = true;
  }

  private resize() {
    const w = this.host.clientWidth;
    const h = Math.max(this.host.clientHeight, 1);
    this.renderer.setSize(w, h);
    this.labels.setSize(w, h);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.requestRender();
  }

  // ---------- point clouds ----------

  setScene(data: SceneData | null) {
    const first = !this.layer && data && data.scans.length > 0;
    if (this.layer) {
      this.scene.remove(this.layer.group);
      this.layer.dispose();
      this.layer = null;
    }
    // The grid marks the project's z = 0 plane under the data.
    this.grid.position.z = -(data?.origin[2] ?? 0);
    if (data && data.scans.length > 0) {
      this.layer = new PointCloudLayer(data, () => this.requestRender());
      this.scene.add(this.layer.group);
      if (first) this.frame(data);
    }
    this.requestRender();
  }

  /** Point the camera at the data. */
  frame(data: SceneData) {
    let r = 1;
    for (const s of data.scans) r = Math.max(r, s.size);
    this.camera.near = Math.max(r / 5000, 0.005);
    this.camera.far = r * 50;
    this.camera.position.set(r * 0.7, -r * 0.9, r * 0.6);
    this.controls.target.set(0, 0, 0);
    this.camera.updateProjectionMatrix();
    this.controls.update();
    this.clipBox.scale.setScalar(r * 0.5);
    this.updateClipUniforms();
  }

  // ---------- built scene ----------

  /** Replace the built 3D objects (already relative to `origin`). */
  setBuilt(group: THREE.Group | null) {
    const moving = this.moving;
    this.attachModel(null, "translate");
    if (this.built) {
      this.scene.remove(this.built);
      this.built.traverse((o) => {
        if (o instanceof THREE.Mesh) {
          o.geometry.dispose();
          for (const m of [o.material].flat()) {
            (m as THREE.MeshStandardMaterial).map?.dispose();
            m.dispose();
          }
        }
      });
    }
    this.built = group;
    if (group) this.scene.add(group);
    if (moving) this.attachModel(moving, this.modelGizmo.mode as "translate" | "rotate");
    this.requestRender();
  }

  /** Put the move gizmo on a placed model (null to remove it). */
  attachModel(id: string | null, mode: "translate" | "rotate") {
    this.moving = null;
    this.modelGizmo.detach();
    const g = id ? this.built?.children.find((c) => c.userData.sceneObject === id) : undefined;
    if (g) {
      // The gizmo edits position and rotation; take them from the stored matrix.
      g.matrix.decompose(g.position, g.quaternion, g.scale);
      g.matrixAutoUpdate = true;
      this.modelGizmo.attach(g);
      this.modelGizmo.setMode(mode);
      this.moving = id;
    }
    this.requestRender();
  }

  /** The camera's position and the orbit target in the project frame (f64). */
  cameraProject(): { eye: [number, number, number]; target: [number, number, number] } {
    const o = this.origin;
    const p = this.camera.position;
    const t = this.controls.target;
    return {
      eye: [p.x + o[0], p.y + o[1], p.z + o[2]],
      target: [t.x + o[0], t.y + o[1], t.z + o[2]],
    };
  }

  get origin(): [number, number, number] {
    return this.layer?.data.origin ?? [0, 0, 0];
  }

  setState(state: StateView) {
    this.layer?.setRevisions(state.revisions);
    this.drawMeasurements(state);
    this.requestRender();
  }

  setColorMode(mode: ColorMode) {
    if (!this.layer) return;
    this.layer.uniforms.uColorMode.value = { rgb: 0, intensity: 1, elevation: 2 }[mode];
    this.requestRender();
  }

  setPointSize(factor: number) {
    if (this.layer) this.layer.uniforms.uSizeFactor.value = factor;
    this.requestRender();
  }

  setEdl(on: boolean, strength: number) {
    this.edl.enabled = on;
    this.edl.strength = strength;
    this.requestRender();
  }

  // ---------- clipping ----------

  setClipBox(mode: ClipMode, gizmo: "translate" | "scale") {
    this.clipMode = mode;
    this.clipBox.visible = mode !== "off";
    if (mode === "off") this.gizmo.detach();
    else {
      this.gizmo.attach(this.clipBox);
      this.gizmo.setMode(gizmo);
    }
    this.updateClipUniforms();
    this.requestRender();
  }

  setClipPlane(on: boolean, axis: 0 | 1 | 2, offset: number, flip: boolean) {
    this.plane = { on, axis, offset, flip };
    if (!this.layer) return;
    const n = [0, 0, 0];
    n[axis] = flip ? -1 : 1;
    // offset is in project coordinates; the GPU works relative to the render origin.
    const local = offset - this.origin[axis];
    this.layer.uniforms.uClipPlane.value.set(n[0], n[1], n[2], -n[axis] * local);
    this.layer.uniforms.uClipPlaneOn.value = on;
    this.requestRender();
  }

  private updateClipUniforms() {
    if (!this.layer) return;
    this.clipBox.updateMatrixWorld();
    this.layer.uniforms.uClipBoxInv.value.copy(this.clipBox.matrixWorld).invert();
    this.layer.uniforms.uClipBox.value = { off: 0, inside: 1, outside: 2 }[this.clipMode];
  }

  /** The clip box as an axis-aligned region in project coordinates, when it is on. */
  clipRegion(): Region | null {
    if (this.clipMode === "off") return null;
    const p = this.clipBox.position;
    const s = this.clipBox.scale;
    const o = this.origin;
    return {
      min: [p.x - s.x / 2 + o[0], p.y - s.y / 2 + o[1], p.z - s.z / 2 + o[2]],
      max: [p.x + s.x / 2 + o[0], p.y + s.y / 2 + o[1], p.z + s.z / 2 + o[2]],
    };
  }

  // ---------- picking ----------

  pick(x: number, y: number): PickOutcome {
    return this.layer
      ? this.layer.pick(this.renderer, this.scene, this.camera, x, y, SNAP_PX)
      : null;
  }

  /** A lasso drawn in CSS pixels, as a cleanup request in NDC with the current camera. */
  lassoRequest(polygon: [number, number][], mode: LassoDepth["mode"]): CleanupRequest {
    const w = this.host.clientWidth;
    const h = this.host.clientHeight;
    this.camera.updateMatrixWorld();
    const vp = new THREE.Matrix4().multiplyMatrices(
      this.camera.projectionMatrix,
      this.camera.matrixWorldInverse,
    );
    return {
      kind: "lasso_delete",
      view_proj: [...vp.elements],
      origin: this.origin,
      polygon: polygon.map(([x, y]) => [(x / w) * 2 - 1, -((y / h) * 2 - 1)]),
      // Defaults documented in docs/methods/cleanup.md.
      depth:
        mode === "all_depths"
          ? { mode }
          : { mode, viewport: [w, h], cell_px: 3, tolerance_m: 0.02, tolerance_rel: 0.005 },
      clip: {
        clip_box: (() => {
          const r = this.clipRegion();
          return r ? [r.min, r.max, this.clipMode === "inside"] : null;
        })(),
        plane: this.plane.on ? [this.plane.axis, this.plane.offset, this.plane.flip] : null,
      },
    };
  }

  // ---------- overlays ----------

  private toRender(p: [number, number, number]) {
    const o = this.origin;
    return new THREE.Vector3(p[0] - o[0], p[1] - o[1], p[2] - o[2]);
  }

  setMarkers(points: [number, number, number][]) {
    this.markers.clear();
    const size = this.camera.position.distanceTo(this.controls.target) * 0.006;
    for (const p of points) {
      const m = new THREE.Mesh(
        new THREE.SphereGeometry(size, 12, 8),
        new THREE.MeshBasicMaterial({ color: 0xf2a53a, depthTest: false }),
      );
      m.position.copy(this.toRender(p));
      m.renderOrder = 10;
      this.markers.add(m);
    }
    if (points.length > 1) this.markers.add(this.polyline(points, 0xf2a53a));
    this.requestRender();
  }

  private polyline(points: [number, number, number][], color: number, closed = false) {
    const pts = points.map((p) => this.toRender(p));
    if (closed) pts.push(pts[0]);
    const line = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints(pts),
      new THREE.LineBasicMaterial({ color, depthTest: false }),
    );
    line.renderOrder = 9;
    return line;
  }

  private drawMeasurements(state: StateView) {
    for (const c of [...this.overlays.children]) {
      c.traverse((o) => o instanceof CSS2DObject && o.element.remove());
    }
    this.overlays.clear();
    for (const m of state.measurements) {
      const pts = m.points.map((p) => p.project);
      const g = new THREE.Group();
      g.add(this.polyline(pts, 0x58a6ff, m.kind === "area"));
      const el = document.createElement("div");
      el.className = "measure-label";
      el.textContent = `#${m.id} ${formatMeasurement(m).label}`;
      const label = new CSS2DObject(el);
      const anchor =
        m.kind === "angle" ? pts[1] : m.kind === "height" ? pts[pts.length - 1] : pts[0];
      label.position.copy(this.toRender(anchor));
      g.add(label);
      this.overlays.add(g);
    }
  }

  /**
   * Orbit the camera around its target for `seconds`, rendering every frame, and report
   * frame times, the adaptive budget and node-selection cost. Used for performance runs.
   */
  benchmark(seconds: number): Promise<Record<string, number>> {
    return new Promise((resolve) => {
      const t = this.controls.target.clone();
      const start = this.camera.position.clone().sub(t);
      const frames: number[] = [];
      const selection: number[] = [];
      const t0 = performance.now();
      let last = t0;
      const step = () => {
        const now = performance.now();
        frames.push(now - last);
        last = now;
        selection.push(this.layer?.lastSelection.ms ?? 0);
        const a = ((now - t0) / 1000) * 0.4;
        this.camera.position.copy(
          start
            .clone()
            .applyAxisAngle(new THREE.Vector3(0, 0, 1), a)
            .add(t),
        );
        this.camera.lookAt(t);
        this.requestRender();
        if (now - t0 < seconds * 1000) requestAnimationFrame(step);
        else {
          const f = frames.slice(1).sort((x, y) => x - y);
          const s = [...selection].sort((x, y) => x - y);
          const avg = f.reduce((x, y) => x + y, 0) / f.length;
          resolve({
            fps: 1000 / avg,
            p95_ms: f[Math.floor(f.length * 0.95)],
            max_ms: f[f.length - 1],
            budget: this.budget.points,
            drawn: this.layer?.loadedPoints ?? 0,
            selection_avg_ms: s.reduce((x, y) => x + y, 0) / s.length,
            selection_max_ms: s[s.length - 1],
            nodes_visited: this.layer?.lastSelection.visited ?? 0,
            nodes_selected: this.layer?.lastSelection.nodes ?? 0,
            frames: f.length,
          });
        }
      };
      requestAnimationFrame(step);
    });
  }

  // ---------- render loop ----------

  private view(): View {
    const cam = this.camera;
    cam.updateMatrixWorld();
    const frustum = new THREE.Frustum().setFromProjectionMatrix(
      new THREE.Matrix4().multiplyMatrices(cam.projectionMatrix, cam.matrixWorldInverse),
    );
    const projScale =
      this.renderer.domElement.height / (2 * Math.tan(THREE.MathUtils.degToRad(cam.fov) / 2));
    let focus: View["focus"];
    if (this.focusActive && this.mouse) {
      const ray = new THREE.Raycaster();
      const w = this.host.clientWidth;
      const h = this.host.clientHeight;
      ray.setFromCamera(
        new THREE.Vector2((this.mouse.x / w) * 2 - 1, -(this.mouse.y / h) * 2 + 1),
        cam,
      );
      focus = {
        origin: ray.ray.origin.toArray() as [number, number, number],
        dir: ray.ray.direction.toArray() as [number, number, number],
        tan: (SNAP_PX * window.devicePixelRatio) / projScale,
      };
    }
    return {
      eye: cam.position.toArray() as [number, number, number],
      frustum: frustum.planes.map((p) => [p.normal.x, p.normal.y, p.normal.z, p.constant]),
      projScale,
      minNodePx: 40,
      budget: this.budget.points,
      focus,
    };
  }

  private loop = () => {
    this.raf = requestAnimationFrame(this.loop);
    if (!this.dirty) {
      this.lastFrame = 0;
      return;
    }
    this.dirty = false;
    const now = performance.now();
    const view = this.view();
    let busy = false;
    if (this.layer) {
      this.layer.uniforms.uScale.value = view.projScale;
      busy = this.layer.update(view);
    }
    this.edl.render(this.renderer, this.scene, this.camera);
    this.labels.render(this.scene, this.camera);
    // Only intervals between back-to-back frames say anything about GPU speed.
    if (this.lastFrame > 0) {
      const ms = now - this.lastFrame;
      this.fps = this.fps * 0.9 + (1000 / ms) * 0.1;
      this.budget = updateBudget(this.budget, ms, this.layer?.lastSelection.limited ?? false);
    }
    this.lastFrame = now;
    if (busy) this.dirty = true;
    const sel = this.layer?.lastSelection;
    this.onStats({
      fps: this.fps,
      drawn: this.layer?.loadedPoints ?? 0,
      budget: this.budget.points,
      selectionMs: sel?.ms ?? 0,
      loading: busy,
    });
  };

  dispose() {
    cancelAnimationFrame(this.raf);
    this.layer?.dispose();
    this.gizmo.dispose();
    this.modelGizmo.dispose();
    this.setBuilt(null);
    this.controls.dispose();
    this.edl.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
    this.labels.domElement.remove();
  }
}
