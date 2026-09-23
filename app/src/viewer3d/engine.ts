// The 3D view: camera, render-on-demand loop, point clouds, EDL, clipping, measurement
// overlays and picking. React only tells it what to show (see Viewport.tsx).
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { TransformControls } from "three/addons/controls/TransformControls.js";
import { CSS2DObject, CSS2DRenderer } from "three/addons/renderers/CSS2DRenderer.js";
import type { CleanupRequest, LassoDepth, Region, SolvedCamera, StateView } from "../api";
import { initialBudget, updateBudget, type BudgetState } from "./budget";
import { EdlPass } from "./edl";
import type { View } from "./lod";
import { formatMeasurement } from "./measureFormat";
import { PointCloudLayer, type ColorMode, type PickOutcome, type SceneData } from "./pointcloud";
import { createScene } from "./scene";
import { maxRadius } from "../tools/camera/model";

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

/** Looking through a solved camera: its pose and lens, and its photo drawn over the scene. */
export interface CameraMatch {
  camera: SolvedCamera;
  photoUrl: string | null;
  opacity: number;
}

// The photo over the scene, through the solved lens: each screen pixel is an undistorted
// image position (the camera's projection, fitted into the view); the photo is sampled where
// the lens model puts that position.
const MATCH_VERTEX = `void main() { gl_Position = vec4(position.xy, 0.0, 1.0); }`;
const MATCH_FRAGMENT = `uniform sampler2D tPhoto;
uniform vec2 uDevice; uniform float uDpr; uniform vec3 uFit; uniform vec3 uK;
uniform vec4 uD; uniform float uP2; uniform vec2 uImage; uniform float uRmax; uniform float uOpacity;
void main() {
  vec2 css = vec2(gl_FragCoord.x, uDevice.y - gl_FragCoord.y) / uDpr;
  vec2 img = (css - uFit.xy) / uFit.z;
  float a = (img.x - uK.y) / uK.x;
  float b = (img.y - uK.z) / uK.x;
  float r2 = a * a + b * b;
  if (sqrt(r2) >= uRmax) discard;
  float radial = 1.0 + uD.x * r2 + uD.y * r2 * r2 + uD.z * r2 * r2 * r2;
  float ad = a * radial + 2.0 * uD.w * a * b + uP2 * (r2 + 2.0 * a * a);
  float bd = b * radial + uD.w * (r2 + 2.0 * b * b) + 2.0 * uP2 * a * b;
  vec2 px = vec2(uK.x * ad + uK.y, uK.x * bd + uK.z);
  if (px.x < 0.0 || px.y < 0.0 || px.x > uImage.x || px.y > uImage.y) discard;
  gl_FragColor = vec4(texture2D(tPhoto, vec2(px.x / uImage.x, 1.0 - px.y / uImage.y)).rgb, uOpacity);
  #include <colorspace_fragment>
}`;

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
  /** An analysis drawn over the scene (a trajectory), relative to the origin. */
  /** Analysis overlays, one per tool. */
  private analysis = new Map<string, THREE.Group>();
  private modelGizmo: TransformControls;
  private moving: string | null = null;
  /** Looking through a solved camera, and the photo drawn over the scene. */
  private match: CameraMatch | null = null;
  private matchScene = new THREE.Scene();
  private matchCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  private matchMaterial: THREE.ShaderMaterial | null = null;
  private matchUrl: string | null = null;
  private normalFov = 50;
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
    if (this.match) this.matchProjection();
    else this.camera.updateProjectionMatrix();
    this.requestRender();
  }

  // ---------- looking through a solved camera ----------

  /** The photo's fit in the view (CSS px): offset and scale of image pixels. */
  private matchFit(c: SolvedCamera): [number, number, number] {
    const w = this.host.clientWidth;
    const h = Math.max(this.host.clientHeight, 1);
    const s = Math.min(w / c.size[0], h / c.size[1]);
    return [(w - c.size[0] * s) / 2, (h - c.size[1] * s) / 2, s];
  }

  /** The solved camera's projection (focal length, principal point), the photo fitted into
   * the view. */
  private matchProjection() {
    const c = this.match!.camera;
    const w = this.host.clientWidth;
    const h = Math.max(this.host.clientHeight, 1);
    const [ox, oy, s] = this.matchFit(c);
    const { near, far } = this.camera;
    const A = (2 * (ox + c.cx * s)) / w - 1;
    const B = 1 - (2 * (oy + c.cy * s)) / h;
    const m = new THREE.Matrix4().set(
      (2 * s * c.f) / w,
      0,
      -A,
      0,
      0,
      (2 * s * c.f) / h,
      -B,
      0,
      0,
      0,
      -(far + near) / (far - near),
      (-2 * far * near) / (far - near),
      0,
      0,
      -1,
      0,
    );
    this.camera.projectionMatrix.copy(m);
    this.camera.projectionMatrixInverse.copy(m.clone().invert());
    // Point sizes and level of detail use the vertical field of view's pixel scale.
    this.camera.fov = THREE.MathUtils.radToDeg(2 * Math.atan(h / (2 * s * c.f)));
    const u = this.matchMaterial?.uniforms;
    if (u) {
      u.uFit.value.set(ox, oy, s);
      const size = this.renderer.getDrawingBufferSize(new THREE.Vector2());
      u.uDevice.value.copy(size);
      u.uDpr.value = size.x / Math.max(w, 1);
    }
  }

  /** Look through a solved camera with its photo over the scene (null to go back to the
   * normal view). Orbiting is off while it is on. */
  setCameraMatch(m: CameraMatch | null) {
    const was = this.match;
    this.match = m;
    if (!m) {
      if (was) {
        this.controls.enabled = true;
        this.camera.fov = this.normalFov;
        this.camera.updateProjectionMatrix();
        const fwd = new THREE.Vector3(...was.camera.rotation[2]);
        this.controls.target.copy(this.camera.position).addScaledVector(fwd, 3);
        this.controls.update();
      }
      this.requestRender();
      return;
    }
    if (!was) this.normalFov = this.camera.fov;
    const c = m.camera;
    const o = this.origin;
    this.controls.enabled = false;
    this.camera.position.set(c.position[0] - o[0], c.position[1] - o[1], c.position[2] - o[2]);
    const [right, down, fwd] = c.rotation.map((r) => new THREE.Vector3(...r));
    const basis = new THREE.Matrix4().makeBasis(right, down.negate(), fwd.negate());
    this.camera.quaternion.setFromRotationMatrix(basis);
    this.camera.updateMatrixWorld();
    if (!this.matchMaterial) {
      this.matchMaterial = new THREE.ShaderMaterial({
        vertexShader: MATCH_VERTEX,
        fragmentShader: MATCH_FRAGMENT,
        uniforms: {
          tPhoto: { value: null },
          uDevice: { value: new THREE.Vector2() },
          uDpr: { value: 1 },
          uFit: { value: new THREE.Vector3() },
          uK: { value: new THREE.Vector3() },
          uD: { value: new THREE.Vector4() },
          uP2: { value: 0 },
          uImage: { value: new THREE.Vector2() },
          uRmax: { value: 1e9 },
          uOpacity: { value: 0.5 },
        },
        transparent: true,
        depthTest: false,
        depthWrite: false,
      });
      this.matchScene.add(new THREE.Mesh(new THREE.PlaneGeometry(2, 2), this.matchMaterial));
    }
    const u = this.matchMaterial.uniforms;
    u.uK.value.set(c.f, c.cx, c.cy);
    const [k1, k2, k3, p1, p2] = c.distortion;
    u.uD.value.set(k1, k2, k3, p1);
    u.uP2.value = p2;
    u.uImage.value.set(c.size[0], c.size[1]);
    u.uRmax.value = Math.min(1e9, maxRadius(c));
    u.uOpacity.value = m.opacity;
    if (m.photoUrl !== this.matchUrl) {
      (u.tPhoto.value as THREE.Texture | null)?.dispose();
      u.tPhoto.value = null;
      this.matchUrl = m.photoUrl;
      if (m.photoUrl) {
        const tex = new THREE.TextureLoader().load(m.photoUrl, () => this.requestRender());
        tex.colorSpace = THREE.SRGBColorSpace;
        u.tPhoto.value = tex;
      }
    }
    this.matchProjection();
    this.requestRender();
  }

  /** Put the view at `eye` looking at `target` (project frame) with a horizontal field of
   * view (degrees); null for the field of view restores the normal one. */
  setView(eye: [number, number, number], target: [number, number, number], hfovDeg: number | null) {
    this.setCameraMatch(null);
    const o = this.origin;
    this.camera.position.set(eye[0] - o[0], eye[1] - o[1], eye[2] - o[2]);
    this.controls.target.set(target[0] - o[0], target[1] - o[1], target[2] - o[2]);
    this.camera.fov =
      hfovDeg === null
        ? this.normalFov
        : THREE.MathUtils.radToDeg(
            2 * Math.atan(Math.tan(THREE.MathUtils.degToRad(hfovDeg) / 2) / this.camera.aspect),
          );
    this.camera.updateProjectionMatrix();
    this.controls.update();
    this.requestRender();
  }

  /** Up to `max` of the drawn scan points (project frame), evenly from each loaded node. */
  samplePoints(max: number): [number, number, number][] {
    const nodes = (this.layer?.group.children ?? []).filter(
      (c): c is THREE.Points => c instanceof THREE.Points && c.visible,
    );
    const total = nodes.reduce((n, c) => n + c.geometry.attributes.position.count, 0);
    const step = Math.max(1, Math.ceil(total / max));
    const o = this.origin;
    const v = new THREE.Vector3();
    const out: [number, number, number][] = [];
    for (const c of nodes) {
      c.updateMatrixWorld();
      const p = c.geometry.attributes.position;
      for (let i = 0; i < p.count; i += step) {
        v.fromBufferAttribute(p, i).applyMatrix4(c.matrixWorld);
        out.push([v.x + o[0], v.y + o[1], v.z + o[2]]);
      }
    }
    return out;
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
    this.applyPoses();
    if (moving) this.attachModel(moving, this.modelGizmo.mode as "translate" | "rotate");
    this.requestRender();
  }

  /** Animated objects' matrices (column-major, relative to `origin`), over their stored ones. */
  private poses = new Map<string, number[]>();

  /** Pose built objects by id for playback; an object left out returns to its stored matrix
   * at the next rebuild. */
  setPoses(poses: Map<string, number[]>) {
    this.poses = poses;
    this.applyPoses();
    this.requestRender();
  }

  private applyPoses() {
    for (const c of this.built?.children ?? []) {
      const m = this.poses.get(c.userData.sceneObject);
      if (m && c.userData.sceneObject !== this.moving) {
        c.matrixAutoUpdate = false;
        c.matrix.fromArray(m);
        c.matrixWorldNeedsUpdate = true;
      }
    }
  }

  /** Replace one tool's analysis overlay (null removes it). */
  setAnalysisOverlay(key: string, group: THREE.Group | null) {
    const old = this.analysis.get(key);
    if (old) {
      this.scene.remove(old);
      old.traverse((o) => {
        if (o instanceof THREE.Mesh || o instanceof THREE.Line) {
          o.geometry.dispose();
          const m = o.material as THREE.Material & { map?: THREE.Texture | null };
          m.map?.dispose();
          m.dispose();
        }
      });
      this.analysis.delete(key);
    }
    if (group) {
      this.analysis.set(key, group);
      this.scene.add(group);
    }
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
    if (this.match && this.matchMaterial?.uniforms.tPhoto.value) {
      this.renderer.autoClear = false;
      this.renderer.render(this.matchScene, this.matchCamera);
      this.renderer.autoClear = true;
    }
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
