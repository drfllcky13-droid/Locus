// Viewer smoke test for CI: the built frontend (app/dist) in headless Chromium with software
// WebGL (SwiftShader), the Tauri IPC and the locus:// node protocol answered from a recorded
// fixture (tools/smoke/fixture, made by capture.mjs). No GPU, no desktop session, no Rust.
//
//   pnpm -C app build && node tools/smoke/viewer-smoke.mjs
//
// Fails on a shader compile error, any console error or uncaught exception, a command the
// fixture doesn't cover, nothing drawn, or a blank canvas (pixels are read back).
import { createRequire } from "node:module";
import { existsSync, readFileSync } from "node:fs";
import { extname } from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = createRequire(new URL("../../app/package.json", import.meta.url))("playwright");
const dist = fileURLToPath(new URL("../../app/dist/", import.meta.url));
const fixture = new URL("./fixture/", import.meta.url);
if (!existsSync(dist)) throw new Error("app/dist is missing: run `pnpm -C app build` first");
const ipc = JSON.parse(readFileSync(new URL("./ipc.json", fixture), "utf8"));

// Stands in for Tauri's injected IPC bridge. Unknown commands fail loudly.
const mock = (ipc) => {
  let next = 1;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(cb, once) {
      const id = next++;
      window[`_${id}`] = (x) => {
        if (once) delete window[`_${id}`];
        cb?.(x);
      };
      return id;
    },
    unregisterCallback(id) {
      delete window[`_${id}`];
    },
    convertFileSrc(path, protocol = "asset") {
      return `http://${protocol}.localhost/${encodeURIComponent(path)}`;
    },
    async invoke(cmd) {
      if (cmd === "plugin:event|listen") return next++;
      if (cmd === "plugin:event|unlisten") return null;
      if (cmd in ipc) return structuredClone(ipc[cmd]);
      console.error(`viewer smoke: the fixture has no answer for "${cmd}"`);
      throw new Error(`no fixture for ${cmd}`);
    },
  };
};

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".png": "image/png" };
const problems = [];
const browser = await chromium.launch({
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});
let failed = false;
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.on("console", (m) => {
    if (m.type() === "error" || /shader error|THREE\.WebGLProgram/i.test(m.text())) problems.push(`${m.type()}: ${m.text().slice(0, 1500)}`);
  });
  page.on("pageerror", (e) => problems.push(`exception: ${e.stack ?? e}`));
  await page.addInitScript(mock, ipc);
  await page.route("http://locus.test/**", (route) => {
    const path = new URL(route.request().url()).pathname.slice(1) || "index.html";
    const file = dist + path;
    if (!existsSync(file)) return route.fulfill({ status: 404 });
    return route.fulfill({ body: readFileSync(file), contentType: types[extname(file)] ?? "application/octet-stream" });
  });
  await page.route("http://locus.localhost/**", (route) => {
    const [, kind, scan, node] = new URL(route.request().url()).pathname.split("/");
    const file = new URL(`./nodes/${scan}_${node}.bin`, fixture);
    if (kind !== "node" || !existsSync(file)) {
      problems.push(`unexpected protocol request ${route.request().url()}`);
      return route.fulfill({ status: 404 });
    }
    return route.fulfill({ body: readFileSync(file), contentType: "application/octet-stream" });
  });

  await page.goto("http://locus.test/");
  await page.waitForFunction(() => globalThis.__locus?.layer?.data.scans.length === 2, null, { timeout: 60_000 });

  const summary = await page.evaluate(async () => {
    const e = globalThis.__locus;
    const gl = e.renderer.getContext();
    const dbg = gl.getExtension("WEBGL_debug_renderer_info");
    const gpu = String(dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER));
    // Read back the next frame the engine draws: our callback runs after its render loop's.
    const nonBackground = () =>
      new Promise((resolve) => {
        e.requestRender();
        requestAnimationFrame(() => {
          const [w, h] = [gl.drawingBufferWidth, gl.drawingBufferHeight];
          const px = new Uint8Array(w * h * 4);
          gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
          let n = 0;
          for (let i = 0; i < px.length; i += 4) {
            const d = Math.abs(px[i] - 0x1e) + Math.abs(px[i + 1] - 0x1f) + Math.abs(px[i + 2] - 0x22);
            if (d > 12) n++;
          }
          resolve(n / (w * h));
        });
      });
    const settle = () => e.benchmark(1);
    e.grid.visible = false; // only points may count as drawn
    const render = await e.benchmark(3);
    const out = { gpu, drawn: render.drawn, frames: render.frames, coverage: {} };
    out.coverage.rgb = await nonBackground();
    for (const mode of ["intensity", "elevation"]) {
      e.setColorMode(mode);
      await settle();
      out.coverage[mode] = await nonBackground();
    }
    e.setColorMode("rgb");
    e.setEdl(false, 1);
    await settle();
    out.coverage.noEdl = await nonBackground();
    e.setEdl(true, 1);
    e.setClipBox("inside", "translate");
    e.setClipPlane(true, 2, 1.0, false);
    await settle();
    out.coverage.clipped = await nonBackground();
    e.setClipBox("off", "translate");
    e.setClipPlane(false, 2, 0, false);
    await settle();
    // GPU picking at the centre of the drawn cloud must identify a point.
    const w = e.host.clientWidth;
    const h = e.host.clientHeight;
    let hit = null;
    for (const [fx, fy] of [[0.5, 0.5], [0.4, 0.6], [0.6, 0.4], [0.5, 0.7], [0.5, 0.3]]) {
      hit = e.pick(fx * w, fy * h);
      if (hit && !("refused" in hit)) break;
    }
    out.pick = hit;
    // Close up: 0.4 m from a drawn point, looking at it. Log depth is negative nearer than
    // 1 m, and the EDL pass once took that for "nothing drawn" and blanked close-ups.
    const node = e.layer.group.children.find((c) => c.visible && c.geometry.attributes.position.count > 0);
    const p = node.geometry.attributes.position;
    const at = e.camera.position.clone().fromBufferAttribute(p, Math.floor(p.count / 2)).applyMatrix4(node.matrixWorld);
    const back = e.camera.position.clone().sub(at).normalize().multiplyScalar(0.4);
    e.camera.position.copy(at).add(back);
    e.controls.target.copy(at);
    e.controls.update();
    await e.benchmark(3);
    out.coverage.closeUp = await nonBackground();
    return out;
  });
  console.log("viewer smoke:", JSON.stringify(summary, null, 1));
  const cov = summary.coverage;
  if (!(summary.drawn > 0)) problems.push("no points drawn");
  for (const [k, v] of Object.entries(cov)) if (k !== "clipped" && !(v > 0.01)) problems.push(`canvas blank with ${k} (${(v * 100).toFixed(2)}% non-background)`);
  if (!summary.pick || "refused" in summary.pick) problems.push(`picking found no point: ${JSON.stringify(summary.pick)}`);
} catch (err) {
  problems.push(String(err?.stack ?? err));
} finally {
  await browser.close();
}
if (problems.length) {
  failed = true;
  console.error(`\n${problems.length} problem(s):\n${problems.join("\n")}`);
}
process.exit(failed ? 1 : 0);
