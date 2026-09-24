// Launch the built Lotus app on a fresh synthetic project with remote debugging on.
// Shared by app-smoke.mjs (full-app check) and capture.mjs (records the viewer fixture).
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { connect } from "./cdp.mjs";

const exe = process.platform === "win32" ? ".exe" : "";

function run(cmd, args) {
  const r = spawnSync(cmd, args, { stdio: "inherit" });
  if (r.status !== 0) throw new Error(`${cmd} ${args[0]} failed`);
}

/**
 * Generate `scans` × `points` synthetic scans (`genArgs`: extra `gen-scene` flags; `room: false`
 * to skip them), import
 * them (with any `extra` evidence files), start the app on the project and connect over CDP. `webviewArgs` are extra WebView2
 * browser arguments. Returns
 * `{ cdp, child, project, close() }`; `close` kills the app and removes the temp folder.
 */
export async function launchApp({ scans = 2, points = 300_000, port = 9223, webviewArgs = "", genArgs = [], extra = [], room = true } = {}) {
  const app = resolve(`target/release/locus${exe}`);
  const validate = resolve(`target/release/locus-validate${exe}`);
  const work = mkdtempSync(join(tmpdir(), "locus-smoke-"));
  const scene = join(work, "room.e57");
  const project = join(work, "smoke.locus");
  // `room: false` skips the synthetic room: only the `extra` files are imported, and `scans`
  // is how many point clouds they hold (what the viewer waits for).
  if (room) run(validate, ["gen-scene", "--scans", String(scans), "--points-per-scan", String(points), "--out", scene, ...genArgs]);
  run(validate, ["import", "--project", project, "--examiner", "Smoke test", ...(room ? [scene] : []), ...extra]);
  const child = spawn(app, [], {
    env: {
      ...process.env,
      LOCUS_OPEN: project,
      LOCUS_EXAMINER: "Smoke test",
      // The env var replaces the app's own browser arguments.
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} ${webviewArgs}`,
    },
    stdio: "inherit",
  });
  const close = () => {
    child.kill();
    try {
      rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
    } catch {
      // the app may still hold files for a moment; the temp dir is disposable
    }
  };
  try {
    const cdp = await connect(port, 120_000);
    // Wait for the point clouds to reach the viewer.
    const deadline = Date.now() + 120_000;
    while ((await cdp.evaluate("globalThis.__locus?.layer?.data.scans.length ?? 0")) < scans) {
      if (Date.now() > deadline) throw new Error("point clouds never appeared in the viewer");
      await new Promise((r) => setTimeout(r, 1000));
    }
    return { cdp, child, project, close };
  } catch (err) {
    close();
    throw err;
  }
}
