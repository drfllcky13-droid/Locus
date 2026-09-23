// Record the viewer fixture used by viewer-smoke.mjs: the backend's answers to the commands
// the viewer makes on startup, and every octree node, from the real app on a small synthetic
// project. Re-run whenever a command's response or the node format changes:
//
//   node tools/smoke/capture.mjs
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { launchApp } from "./app.mjs";

const out = new URL("./fixture/", import.meta.url);
const app = await launchApp({ scans: 2, points: 60_000 });
try {
  const ipc = await app.cdp.evaluate(`(async () => {
    const inv = window.__TAURI_INTERNALS__.invoke;
    const startup = await inv("startup");
    return {
      startup,
      app_info: await inv("app_info"),
      project_open: await inv("project_open", { root: startup.open, examinerName: startup.examiner, onProgress: "__CHANNEL__:" + window.__TAURI_INTERNALS__.transformCallback(() => {}) }).catch(() => null),
      scene_view: await inv("scene_view"),
      analysis_state: await inv("analysis_state"),
      diagrams: await inv("diagrams"),
      scenes: await inv("scenes"),
    };
  })()`);
  if (!ipc.project_open) throw new Error("project_open could not be recorded");
  rmSync(out, { recursive: true, force: true });
  mkdirSync(new URL("./nodes/", out), { recursive: true });
  let bytes = 0;
  for (const scan of ipc.scene_view.scans) {
    const key = scan.key;
    for (let n = 0; n < scan.nodes.length; n++) {
      const b64 = await app.cdp.evaluate(`fetch("http://locus.localhost/node/${key}/${n}").then(r => r.arrayBuffer()).then(b => { let s = ""; const u = new Uint8Array(b); for (let i = 0; i < u.length; i++) s += String.fromCharCode(u[i]); return btoa(s); })`);
      const buf = Buffer.from(b64, "base64");
      bytes += buf.length;
      writeFileSync(new URL(`./nodes/${key}_${n}.bin`, out), buf);
    }
  }
  writeFileSync(new URL("./ipc.json", out), JSON.stringify(ipc, null, 1) + "\n");
  console.log(`fixture: ${ipc.scene_view.scans.length} scans, ${(bytes / 1e6).toFixed(2)} MB of nodes`);
} finally {
  app.cdp.close();
  app.close();
}
