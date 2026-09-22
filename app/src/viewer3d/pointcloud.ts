// Streams octree nodes from Rust and draws them. Positions reach the GPU as f32 relative
// to each node; each node's matrix is computed here in f64 (JS numbers) relative to the
// render origin, so georeferenced coordinates never become f32 on the GPU.
import { convertFileSrc } from "@tauri-apps/api/core";
import * as THREE from "three";
import { selectNodes, type LodScan, type View } from "./lod";
import { MAX_SLOTS, PICK_GLSL, nearest } from "./pick";

export interface NodeData {
  name: string;
  min: [number, number, number];
  size: number;
  count: number;
  spacing: number;
  children: number;
}

export interface ScanData {
  key: string;
  name: string;
  /** Row-major 4×4, scan-local → project. */
  pose: number[];
  min: [number, number, number];
  size: number;
  points: number;
  has_color: boolean;
  has_intensity: boolean;
  nodes: NodeData[];
}

export interface SceneData {
  origin: [number, number, number];
  scans: ScanData[];
}

export type ColorMode = "rgb" | "intensity" | "elevation";

export interface PickHit {
  scan: string;
  node: number;
  k: number;
  revision: number;
}

/** Most nodes kept in GPU memory: about 2× the largest budget. */
const LOADED_POINTS_MAX = 60_000_000;
/** Spike item 2: cap the bytes handed to the GPU per frame. */
const UPLOAD_BYTES_PER_FRAME = 4 << 20;
/** Spike item 3: four node requests in flight. */
const IN_FLIGHT = 4;

interface Loaded {
  points: THREE.Points;
  material: THREE.ShaderMaterial;
  pickMaterial: THREE.ShaderMaterial;
  bytes: number;
  count: number;
  revision: number;
  slot: number;
  lastUsed: number;
}

const VERTEX = `
in vec3 aColor;
in float aIntensity;
uniform float uSpacing;
uniform float uScale;
uniform float uSizeFactor;
uniform int uColorMode;
uniform bool uHasColor;
uniform bool uHasIntensity;
uniform vec2 uElevation;
uniform float uOriginZ;
uniform mat4 uClipBoxInv;
uniform int uClipBox;
uniform vec4 uClipPlane;
uniform bool uClipPlaneOn;
out vec3 vColor;
flat out float vIndex;

vec3 ramp(float t) {
  t = clamp(t, 0.0, 1.0);
  return clamp(vec3(1.5 - abs(4.0 * t - 3.0), 1.5 - abs(4.0 * t - 2.0), 1.5 - abs(4.0 * t - 1.0)), 0.0, 1.0);
}

void main() {
  vec4 world = modelMatrix * vec4(position, 1.0);
  bool clipped = false;
  if (uClipBox != 0) {
    vec3 b = (uClipBoxInv * world).xyz;
    bool inside = all(lessThanEqual(abs(b), vec3(0.5)));
    clipped = (uClipBox == 1) ? !inside : inside;
  }
  if (uClipPlaneOn && dot(uClipPlane.xyz, world.xyz) + uClipPlane.w < 0.0) clipped = true;
  vec4 mv = viewMatrix * world;
  gl_Position = clipped ? vec4(2.0, 2.0, 2.0, 1.0) : projectionMatrix * mv;
  gl_PointSize = clamp(uSizeFactor * uSpacing * uScale / max(-mv.z, 1e-6), 1.5, 8.0);
  vIndex = float(gl_VertexID);
  if (uColorMode == 0 && uHasColor) vColor = aColor;
  else if (uColorMode == 1 && uHasIntensity) vColor = vec3(pow(aIntensity, 0.6));
  else vColor = ramp((world.z + uOriginZ - uElevation.x) / max(uElevation.y - uElevation.x, 1e-6));
}`;

const FRAGMENT = `
in vec3 vColor;
void main() {
  vec2 c = gl_PointCoord - 0.5;
  if (dot(c, c) > 0.25) discard;
  gl_FragColor = vec4(vColor, 1.0); // sRGB values straight through; see edl.ts
}`;

const PICK_FRAGMENT = `
flat in float vIndex;
uniform float uSlot;
${PICK_GLSL}
void main() {
  vec2 c = gl_PointCoord - 0.5;
  if (dot(c, c) > 0.25) discard;
  gl_FragColor = encodePick(uSlot, vIndex);
}`;

/** Row-major pose → three.js matrix for a node: R·(p + nodeMin) + t − origin. */
function nodeMatrix(pose: number[], nodeMin: number[], origin: number[]): THREE.Matrix4 {
  const r = (i: number, j: number) => pose[i * 4 + j];
  const t = [0, 1, 2].map(
    (i) => r(i, 0) * nodeMin[0] + r(i, 1) * nodeMin[1] + r(i, 2) * nodeMin[2] + r(i, 3) - origin[i],
  );
  return new THREE.Matrix4().set(
    r(0, 0),
    r(0, 1),
    r(0, 2),
    t[0],
    r(1, 0),
    r(1, 1),
    r(1, 2),
    t[1],
    r(2, 0),
    r(2, 1),
    r(2, 2),
    t[2],
    0,
    0,
    0,
    1,
  );
}

export class PointCloudLayer {
  readonly group = new THREE.Group();
  readonly data: SceneData;
  private lod: LodScan[];
  private loaded = new Map<string, Loaded>();
  private loading = new Set<string>();
  private pending: { key: string; entry: Loaded }[] = [];
  private wanted: string[] = [];
  private slots: (string | null)[] = [null];
  private frame = 0;
  private revisions: Record<string, number> = {};
  /** Shared by every node's material; change `.value` to update all. */
  readonly uniforms = {
    uScale: { value: 1000 },
    uSizeFactor: { value: 1.0 },
    uColorMode: { value: 0 },
    uElevation: { value: new THREE.Vector2(0, 1) },
    uOriginZ: { value: 0 },
    uClipBoxInv: { value: new THREE.Matrix4() },
    uClipBox: { value: 0 },
    uClipPlane: { value: new THREE.Vector4(0, 0, 1, 0) },
    uClipPlaneOn: { value: false },
  };
  private base = convertFileSrc("", "locus");
  lastSelection = { points: 0, limited: false, visited: 0, ms: 0, nodes: 0 };

  constructor(
    data: SceneData,
    private onChange: () => void,
  ) {
    this.data = data;
    this.uniforms.uOriginZ.value = data.origin[2];
    this.lod = data.scans.map((s) => {
      const index = new Map(s.nodes.map((n, i) => [n.name, i]));
      return {
        root: index.get("r") ?? 0,
        nodes: s.nodes.map((n) => {
          const local = n.min.map((v) => v + n.size / 2);
          const c = [0, 1, 2].map(
            (i) =>
              s.pose[i * 4] * local[0] +
              s.pose[i * 4 + 1] * local[1] +
              s.pose[i * 4 + 2] * local[2] +
              s.pose[i * 4 + 3] -
              data.origin[i],
          ) as [number, number, number];
          const children: number[] = [];
          for (let o = 0; o < 8; o++) {
            if (n.children & (1 << o)) {
              const ci = index.get(n.name + o);
              if (ci !== undefined) children.push(ci);
            }
          }
          return { center: c, radius: (n.size * Math.sqrt(3)) / 2, count: n.count, children };
        }),
      };
    });
    // Elevation colours span the data's height range in the project frame.
    let lo = Infinity;
    let hi = -Infinity;
    for (const s of data.scans) {
      const z0 = s.pose[11] + s.pose[10] * s.min[2];
      lo = Math.min(lo, z0, z0 + s.pose[10] * s.size);
      hi = Math.max(hi, z0, z0 + s.pose[10] * s.size);
    }
    if (Number.isFinite(lo)) this.uniforms.uElevation.value.set(lo, hi);
  }

  setRevisions(revisions: Record<string, number>) {
    const changed = Object.keys(revisions).filter((k) => (this.revisions[k] ?? 0) !== revisions[k]);
    this.revisions = { ...revisions };
    // Drop nodes drawn before a cleanup changed their scan; they reload on the next update.
    for (const [key, entry] of this.loaded) {
      const scan = key.split("/")[0];
      if (changed.includes(scan) && entry.revision !== revisions[scan]) this.unload(key);
    }
    this.pending = this.pending.filter(({ key, entry }) => {
      const stale = entry.revision !== (revisions[key.split("/")[0]] ?? 0);
      if (stale) {
        entry.points.geometry.dispose();
        entry.material.dispose();
        entry.pickMaterial.dispose();
        if (entry.slot) this.slots[entry.slot] = null;
      }
      return !stale;
    });
    if (changed.length) this.onChange();
  }

  /** Choose nodes for this view, start loads, and hand a budgeted amount to the GPU. */
  update(view: View): boolean {
    this.frame++;
    const t0 = performance.now();
    const sel = selectNodes(this.lod, view);
    this.lastSelection = {
      points: sel.points,
      limited: sel.limited,
      visited: sel.visited,
      ms: performance.now() - t0,
      nodes: sel.nodes.length,
    };

    const visible = new Set<string>();
    this.wanted = [];
    for (const [si, ni] of sel.nodes) {
      const key = `${this.data.scans[si].key}/${ni}`;
      visible.add(key);
      const entry = this.loaded.get(key);
      if (entry) entry.lastUsed = this.frame;
      else if (!this.loading.has(key)) this.wanted.push(key);
    }
    for (const [key, entry] of this.loaded) entry.points.visible = visible.has(key);
    this.pump();

    let uploaded = 0;
    while (this.pending.length && uploaded < UPLOAD_BYTES_PER_FRAME) {
      const { key, entry } = this.pending.shift()!;
      this.loaded.set(key, entry);
      entry.lastUsed = this.frame;
      entry.points.visible = visible.has(key);
      this.group.add(entry.points);
      uploaded += entry.bytes;
    }
    this.evict(visible);
    return this.pending.length > 0 || this.loading.size > 0 || this.wanted.length > 0;
  }

  private pump() {
    while (this.loading.size < IN_FLIGHT && this.wanted.length) {
      const key = this.wanted.shift()!;
      this.loading.add(key);
      void this.load(key).finally(() => {
        this.loading.delete(key);
        this.onChange();
      });
    }
  }

  private async load(key: string) {
    const [scanKey, nodeStr] = key.split("/");
    const scan = this.data.scans.find((s) => s.key === scanKey);
    if (!scan) return;
    const node = scan.nodes[Number(nodeStr)];
    const revision = this.revisions[scanKey] ?? 0;
    const res = await fetch(`${this.base}node/${scanKey}/${nodeStr}?r=${revision}`);
    if (!res.ok) return;
    const buf = await res.arrayBuffer();
    if ((this.revisions[scanKey] ?? 0) !== revision) return; // a cleanup landed meanwhile
    const view = new DataView(buf);
    const n = view.getUint32(0, true);
    const flags = view.getUint32(4, true);
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(buf, 16, n * 3), 3));
    let at = 16 + n * 12;
    if (flags & 2) {
      g.setAttribute("aIntensity", new THREE.BufferAttribute(new Uint16Array(buf, at, n), 1, true));
      at = Math.ceil((at + n * 2) / 4) * 4;
    }
    if (flags & 1)
      g.setAttribute("aColor", new THREE.BufferAttribute(new Uint8Array(buf, at, n * 3), 3, true));
    const h = node.size / 2;
    g.boundingSphere = new THREE.Sphere(new THREE.Vector3(h, h, h), h * Math.sqrt(3));

    const slot = this.takeSlot(key);
    const own = {
      uSpacing: { value: node.spacing },
      uHasColor: { value: Boolean(flags & 1) },
      uHasIntensity: { value: Boolean(flags & 2) },
      uSlot: { value: slot },
    };
    const make = (fragmentShader: string) =>
      new THREE.ShaderMaterial({
        glslVersion: THREE.GLSL3,
        vertexShader: VERTEX,
        fragmentShader,
        uniforms: { ...this.uniforms, ...own },
      });
    const material = make(FRAGMENT);
    const pickMaterial = make(PICK_FRAGMENT);
    const points = new THREE.Points(g, material);
    points.matrixAutoUpdate = false;
    points.matrix.copy(nodeMatrix(scan.pose, node.min, this.data.origin));
    points.matrixWorldNeedsUpdate = true;
    points.frustumCulled = false;
    this.pending.push({
      key,
      entry: {
        points,
        material,
        pickMaterial,
        bytes: buf.byteLength,
        count: n,
        revision,
        slot,
        lastUsed: this.frame,
      },
    });
  }

  private takeSlot(key: string): number {
    let i = this.slots.indexOf(null, 1);
    if (i < 0) {
      if (this.slots.length > MAX_SLOTS) return 0; // not pickable; eviction frees slots
      i = this.slots.length;
      this.slots.push(null);
    }
    this.slots[i] = key;
    return i;
  }

  private unload(key: string) {
    const e = this.loaded.get(key);
    if (!e) return;
    this.group.remove(e.points);
    e.points.geometry.dispose();
    e.material.dispose();
    e.pickMaterial.dispose();
    if (e.slot) this.slots[e.slot] = null;
    this.loaded.delete(key);
  }

  private evict(visible: Set<string>) {
    let total = 0;
    for (const e of this.loaded.values()) total += e.count;
    if (total <= LOADED_POINTS_MAX && this.slots.length <= MAX_SLOTS) return;
    const old = [...this.loaded]
      .filter(([k]) => !visible.has(k))
      .sort((a, b) => a[1].lastUsed - b[1].lastUsed);
    for (const [key, e] of old) {
      if (total <= LOADED_POINTS_MAX * 0.8) break;
      total -= e.count;
      this.unload(key);
    }
  }

  get loadedPoints(): number {
    let n = 0;
    for (const e of this.loaded.values()) if (e.points.visible) n += e.count;
    return n;
  }

  /**
   * Identify the point nearest the cursor within `radius` CSS px, drawn exactly as on
   * screen (same size, clipping). Only an id comes back; Rust resolves the coordinates.
   */
  pick(
    renderer: THREE.WebGLRenderer,
    scene: THREE.Scene,
    camera: THREE.PerspectiveCamera,
    x: number,
    y: number,
    radius: number,
  ): PickHit | null {
    const dpr = renderer.getPixelRatio();
    const size = Math.round(radius * dpr) * 2 + 1;
    const full = renderer.getDrawingBufferSize(new THREE.Vector2());
    const rt = new THREE.WebGLRenderTarget(size, size);
    const hidden = scene.children.filter((c) => c !== this.group && c.visible);
    hidden.forEach((c) => (c.visible = false));
    const background = scene.background;
    scene.background = null;
    for (const e of this.loaded.values()) e.points.material = e.pickMaterial;
    camera.setViewOffset(
      full.x,
      full.y,
      Math.round(x * dpr) - (size - 1) / 2,
      Math.round(y * dpr) - (size - 1) / 2,
      size,
      size,
    );
    const clear = renderer.getClearColor(new THREE.Color());
    const alpha = renderer.getClearAlpha();
    renderer.setClearColor(0x000000, 0);
    renderer.setRenderTarget(rt);
    renderer.clear();
    renderer.render(scene, camera);
    const px = new Uint8Array(size * size * 4);
    renderer.readRenderTargetPixels(rt, 0, 0, size, size, px);
    renderer.setRenderTarget(null);
    renderer.setClearColor(clear, alpha);
    camera.clearViewOffset();
    for (const e of this.loaded.values()) e.points.material = e.material;
    scene.background = background;
    hidden.forEach((c) => (c.visible = true));
    rt.dispose();

    // readPixels rows run bottom-up; flip so row 0 is the top of the window.
    const flipped = new Uint8Array(px.length);
    for (let row = 0; row < size; row++)
      flipped.set(
        px.subarray((size - 1 - row) * size * 4, (size - row) * size * 4),
        row * size * 4,
      );
    const hit = nearest(flipped, size);
    if (!hit) return null;
    const key = this.slots[hit.slot];
    const entry = key ? this.loaded.get(key) : undefined;
    if (!key || !entry) return null;
    const [scan, node] = key.split("/");
    return { scan, node: Number(node), k: hit.index, revision: entry.revision };
  }

  dispose() {
    for (const key of [...this.loaded.keys()]) this.unload(key);
    this.pending = [];
  }
}
