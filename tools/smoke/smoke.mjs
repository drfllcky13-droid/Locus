// End-to-end smoke test of the built app's viewer, on software WebGL (SwiftShader), so it
// runs on CI machines without a GPU. Fails on any shader compile error, console error or
// uncaught exception, or on any failed check in tools.js.
//
//   node tools/smoke/smoke.mjs [--app target/release/locus.exe]
//                              [--validate target/release/locus-validate.exe]
//
// Steps: generate a small synthetic scene, import it into a fresh project with the app's
// own import path, launch the app on that project with remote debugging, wait for the point
// clouds, run tools.js in the page, and collect every console message along the way.
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { connect } from "./cdp.mjs";

const arg = (name, fallback) => {
  const i = process.argv.indexOf(name);
  return i > 0 ? process.argv[i + 1] : fallback;
};
const exe = process.platform === "win32" ? ".exe" : "";
const app = resolve(arg("--app", `target/release/locus${exe}`));
const validate = resolve(arg("--validate", `target/release/locus-validate${exe}`));
const port = 9223;
const work = mkdtempSync(join(tmpdir(), "locus-smoke-"));
const scene = join(work, "room.e57");
const project = join(work, "smoke.locus");

function run(cmd, args) {
  const r = spawnSync(cmd, args, { stdio: "inherit" });
  if (r.status !== 0) throw new Error(`${cmd} ${args[0]} failed`);
}

let child;
let failed = false;
try {
  run(validate, ["gen-scene", "--scans", "2", "--points-per-scan", "300000", "--out", scene]);
  run(validate, ["import", "--project", project, "--examiner", "Smoke test", scene]);

  child = spawn(app, [], {
    env: {
      ...process.env,
      LOCUS_OPEN: project,
      LOCUS_EXAMINER: "Smoke test",
      // Software WebGL: no GPU needed. The env var replaces the app's own browser args.
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --use-angle=swiftshader --enable-unsafe-swiftshader --ignore-gpu-blocklist`,
    },
    stdio: "inherit",
  });

  const cdp = await connect(port, 120_000);
  const problems = [];
  cdp.on((m) => {
    if (m.method === "Console.messageAdded") {
      const { level, text } = m.params.message;
      if (level === "error" || /shader error|THREE\.WebGLProgram/i.test(text)) problems.push(`${level}: ${text.slice(0, 1500)}`);
    }
    if (m.method === "Runtime.exceptionThrown") problems.push(`exception: ${JSON.stringify(m.params.exceptionDetails).slice(0, 1500)}`);
  });
  await cdp.send("Console.enable"); // replays messages logged before we connected
  await cdp.send("Runtime.enable");

  // Wait for both scans' octrees to load into the viewer.
  const deadline = Date.now() + 120_000;
  while ((await cdp.evaluate("globalThis.__locus?.layer?.data.scans.length ?? 0")) < 2) {
    if (Date.now() > deadline) throw new Error("point clouds never appeared in the viewer");
    await new Promise((r) => setTimeout(r, 1000));
  }

  const summary = await cdp.evaluate(readFileSync(new URL("./tools.js", import.meta.url), "utf8"));
  console.log("smoke checks passed:", JSON.stringify(summary, null, 1));
  await new Promise((r) => setTimeout(r, 1000)); // let late console messages arrive
  cdp.close();
  if (problems.length) {
    failed = true;
    console.error(`\n${problems.length} console problem(s):\n${problems.join("\n")}`);
  }
} catch (err) {
  failed = true;
  console.error(String(err?.stack ?? err));
} finally {
  child?.kill();
  try {
    rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
  } catch {
    // the app may still hold files for a moment; the temp dir is disposable
  }
}
process.exit(failed ? 1 : 0);
