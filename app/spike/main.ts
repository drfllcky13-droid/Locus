// Phase 2 rendering spike: can the webview stream and draw a large point cloud fast enough?
// Measures chunk transfer, frame times while chunks stream in, and steady-state frame rate
// at several point budgets with eye-dome lighting (EDL) off and on. Results go to
// target/fixtures/spike-results-<label>.json through the `spike_report` command.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import * as THREE from "three";

const POINTS = 1_000_000;
const CHUNKS = 50;
const COLUMNS = 10;
const CHUNK_BYTES = POINTS * 15;
const BUDGETS = [1, 5, 10, 20, 30, 50]; // million points drawn per frame

const params = new URLSearchParams(await invoke<string>("spike_params"));
const power = (params.get("power") ?? "high-performance") as WebGLPowerPreference;
const label = params.get("label") ?? power;

const status = document.getElementById("status")!;
const lines: string[] = [];
const say = (s: string) => {
  lines.push(s);
  status.textContent = lines.slice(-24).join("\n");
};

// ---------- renderer, scene, EDL ----------

const renderer = new THREE.WebGLRenderer({ antialias: false, powerPreference: power });
renderer.setPixelRatio(window.devicePixelRatio);
renderer.setSize(innerWidth, innerHeight);
document.body.prepend(renderer.domElement);
const width = Math.floor(innerWidth * devicePixelRatio);
const height = Math.floor(innerHeight * devicePixelRatio);

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x1e1f22);
const camera = new THREE.PerspectiveCamera(50, innerWidth / innerHeight, 0.5, 1000);
camera.up.set(0, 0, 1);
const center = new THREE.Vector3(50, 25, 0);
function orbit(ms: number) {
  const a = ms * 0.0004;
  camera.position.set(center.x + 75 * Math.cos(a), center.y + 75 * Math.sin(a), 40);
  camera.lookAt(center);
}

const pointMaterial = new THREE.PointsMaterial({
  size: 2,
  sizeAttenuation: false,
  vertexColors: true,
});

const target = new THREE.WebGLRenderTarget(width, height, {
  depthTexture: new THREE.DepthTexture(width, height),
});
const edl = new THREE.ShaderMaterial({
  uniforms: {
    tColor: { value: target.texture },
    tDepth: { value: target.depthTexture },
    texel: { value: new THREE.Vector2(1 / width, 1 / height) },
    near: { value: camera.near },
    far: { value: camera.far },
  },
  vertexShader: `varying vec2 vUv;
    void main() { vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }`,
  // Eight-neighbour log-depth obscurance, as in the usual point-cloud EDL formulation.
  fragmentShader: `uniform sampler2D tColor; uniform sampler2D tDepth;
    uniform vec2 texel; uniform float near; uniform float far;
    varying vec2 vUv;
    float logDepth(vec2 uv) {
      float d = texture2D(tDepth, uv).x;
      if (d >= 1.0) return -1.0;
      float z = d * 2.0 - 1.0;
      return log2(2.0 * near * far / (far + near - z * (far - near)));
    }
    void main() {
      vec4 color = texture2D(tColor, vUv);
      float dc = logDepth(vUv);
      if (dc < 0.0) { gl_FragColor = color; return; }
      float sum = 0.0;
      for (int i = 0; i < 8; i++) {
        float a = float(i) * 0.7853982;
        float dn = logDepth(vUv + vec2(cos(a), sin(a)) * texel * 1.4);
        sum += dn < 0.0 ? 1.0 : max(0.0, dc - dn);
      }
      gl_FragColor = vec4(color.rgb * exp(-sum * 40.0 / 8.0), 1.0);
    }`,
  depthTest: false,
  depthWrite: false,
});
const quadScene = new THREE.Scene();
quadScene.add(new THREE.Mesh(new THREE.PlaneGeometry(2, 2), edl));
const quadCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);

function render(useEdl: boolean) {
  if (!useEdl) return renderer.render(scene, camera);
  renderer.setRenderTarget(target);
  renderer.render(scene, camera);
  renderer.setRenderTarget(null);
  renderer.render(quadScene, quadCamera);
}

// ---------- chunks ----------

const chunks: THREE.Points[] = [];

function chunkObject(i: number, buf: ArrayBuffer): THREE.Points {
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(buf, 0, POINTS * 3), 3));
  g.setAttribute(
    "color",
    new THREE.BufferAttribute(new Uint8Array(buf, POINTS * 12, POINTS * 3), 3, true),
  );
  const ox = (i % COLUMNS) * 10;
  const oy = Math.floor(i / COLUMNS) * 10;
  g.boundingSphere = new THREE.Sphere(new THREE.Vector3(ox + 5, oy + 5, 1), 9);
  return new THREE.Points(g, pointMaterial);
}

const fetchChunk = (i: number) =>
  fetch(convertFileSrc(String(i), "spike")).then((r) => {
    if (!r.ok) throw new Error(`chunk ${i}: HTTP ${r.status}`);
    return r.arrayBuffer();
  });
const invokeChunk = (i: number) => invoke<ArrayBuffer>("spike_chunk", { i });

// ---------- measurement ----------

interface FrameStats {
  frames: number;
  fps: number;
  avg_ms: number;
  p95_ms: number;
  max_ms: number;
  over_50ms: number;
}

function stats(times: number[]): FrameStats {
  const sorted = [...times].sort((a, b) => a - b);
  const sum = times.reduce((a, b) => a + b, 0);
  const round = (v: number) => Math.round(v * 100) / 100;
  return {
    frames: times.length,
    fps: round((1000 * times.length) / sum),
    avg_ms: round(sum / times.length),
    p95_ms: round(sorted[Math.floor(sorted.length * 0.95)] ?? 0),
    max_ms: round(sorted[sorted.length - 1] ?? 0),
    over_50ms: times.filter((t) => t > 50).length,
  };
}

/** Render continuously for `seconds`, returning per-frame intervals. */
function run(seconds: number, useEdl: boolean): Promise<number[]> {
  return new Promise((resolve) => {
    const times: number[] = [];
    let start = -1;
    let last = 0;
    const frame = (now: number) => {
      if (start < 0) start = now;
      else times.push(now - last);
      last = now;
      orbit(now);
      render(useEdl);
      if (now - start < seconds * 1000) requestAnimationFrame(frame);
      else resolve(times);
    };
    requestAnimationFrame(frame);
  });
}

async function transfer(name: string, get: (i: number) => Promise<ArrayBuffer>) {
  const ms: number[] = [];
  for (let i = 0; i < 10; i++) {
    const t = performance.now();
    const buf = await get(i);
    ms.push(performance.now() - t);
    if (buf.byteLength !== CHUNK_BYTES)
      throw new Error(`${name}: chunk ${i} is ${buf.byteLength} bytes`);
  }
  ms.sort((a, b) => a - b);
  const median = ms[5];
  const result = {
    median_ms_per_chunk: Math.round(median * 10) / 10,
    median_mb_per_s: Math.round(CHUNK_BYTES / 1e6 / (median / 1000)),
    fastest_ms: Math.round(ms[0] * 10) / 10,
    slowest_ms: Math.round(ms[9] * 10) / 10,
  };
  say(`${name}: ${result.median_mb_per_s} MB/s (${result.median_ms_per_chunk} ms per 15 MB chunk)`);
  return result;
}

/** Load every chunk (4 requests in flight) while rendering, and record the frame times. */
async function stream() {
  const t0 = performance.now();
  let firstVisible = 0;
  let loaded = 0;
  let rendering = true;
  const times: number[] = [];
  const loop = new Promise<void>((resolve) => {
    let last = -1;
    const frame = (now: number) => {
      if (last >= 0) times.push(now - last);
      last = now;
      orbit(now);
      render(true);
      if (rendering) requestAnimationFrame(frame);
      else resolve();
    };
    requestAnimationFrame(frame);
  });
  let next = 0;
  const worker = async () => {
    while (next < CHUNKS) {
      const i = next++;
      const obj = chunkObject(i, await fetchChunk(i));
      chunks[i] = obj;
      scene.add(obj);
      loaded++;
      if (loaded === 1) firstVisible = performance.now() - t0;
      if (loaded % 10 === 0) say(`streamed ${loaded}/${CHUNKS} chunks`);
    }
  };
  await Promise.all([worker(), worker(), worker(), worker()]);
  // Let the last uploads reach the GPU before stopping the clock.
  await new Promise((r) => setTimeout(r, 500));
  rendering = false;
  await loop;
  const total = performance.now() - t0 - 500;
  return {
    total_ms: Math.round(total),
    first_chunk_visible_ms: Math.round(firstVisible),
    effective_mb_per_s: Math.round((CHUNKS * CHUNK_BYTES) / 1e6 / (total / 1000)),
    frames_during_stream: stats(times),
  };
}

async function environment() {
  const gl = renderer.getContext();
  const dbg = gl.getExtension("WEBGL_debug_renderer_info");
  const gpu = "gpu" in navigator ? (navigator as unknown as { gpu: GpuLike }).gpu : null;
  const adapter = gpu ? await gpu.requestAdapter({ powerPreference: power }) : null;
  return {
    user_agent: navigator.userAgent,
    webgl_renderer: dbg
      ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)
      : gl.getParameter(gl.RENDERER),
    webgl_version: gl.getParameter(gl.VERSION),
    power_preference: power,
    canvas_px: [width, height],
    device_pixel_ratio: devicePixelRatio,
    webgpu_available: Boolean(adapter),
    webgpu_adapter: adapter?.info ? { ...adapter.info } : null,
  };
}

interface GpuLike {
  requestAdapter(o: { powerPreference: string }): Promise<{ info?: Record<string, string> } | null>;
}

async function main() {
  const env = await environment();
  say(`GPU: ${env.webgl_renderer}\ncanvas ${width}x${height}, WebGPU ${env.webgpu_available}`);

  const fetchResult = await transfer("custom protocol fetch", fetchChunk);
  const invokeResult = await transfer("binary invoke", invokeChunk);

  say("streaming 50M points while rendering with EDL…");
  const streaming = await stream();
  say(
    `stream: ${streaming.total_ms} ms total, worst frame ${streaming.frames_during_stream.max_ms} ms, ` +
      `${streaming.frames_during_stream.over_50ms} frames over 50 ms`,
  );

  const budgets = [];
  for (const b of BUDGETS) {
    chunks.forEach((c, i) => (c.visible = i < b));
    const row: Record<string, unknown> = { million_points: b };
    for (const useEdl of [false, true]) {
      await run(0.7, useEdl); // warm-up
      const s = stats(await run(4, useEdl));
      row[useEdl ? "edl" : "plain"] = s;
      say(`${b}M points, EDL ${useEdl ? "on " : "off"}: ${s.fps} fps, p95 ${s.p95_ms} ms`);
    }
    budgets.push(row);
  }

  // An octree draws many small nodes, not a few big chunks. Same points, split into
  // 20k-point pieces that share each chunk's GPU buffers, to expose per-draw overhead.
  const PIECE = 20_000;
  chunks.forEach((c) => (c.visible = false));
  const pieces: THREE.Points[] = [];
  for (let i = 0; i < CHUNKS; i++) {
    const src = chunks[i].geometry;
    for (let start = 0; start < POINTS; start += PIECE) {
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", src.getAttribute("position"));
      g.setAttribute("color", src.getAttribute("color"));
      g.setDrawRange(start, PIECE);
      g.boundingSphere = src.boundingSphere;
      const p = new THREE.Points(g, pointMaterial);
      p.visible = false;
      pieces.push(p);
      scene.add(p);
    }
  }
  const perChunk = POINTS / PIECE;
  const fragmented = [];
  for (const b of [10, 20]) {
    pieces.forEach((p, k) => (p.visible = k < b * perChunk));
    const row: Record<string, unknown> = { million_points: b, draw_calls: b * perChunk };
    for (const useEdl of [false, true]) {
      await run(0.7, useEdl);
      const s = stats(await run(4, useEdl));
      row[useEdl ? "edl" : "plain"] = s;
      say(`${b}M points in ${b * perChunk} draws, EDL ${useEdl ? "on " : "off"}: ${s.fps} fps`);
    }
    fragmented.push(row);
  }

  const result = {
    label,
    environment: env,
    transfer: { fetch: fetchResult, invoke: invokeResult },
    streaming,
    budgets,
    fragmented,
  };
  say("done; writing results");
  await invoke("spike_report", { label, json: JSON.stringify(result, null, 2) });
}

main().catch((e) => {
  say(`FAILED: ${e}`);
  void invoke("spike_report", { label, json: JSON.stringify({ label, error: String(e) }) });
});
