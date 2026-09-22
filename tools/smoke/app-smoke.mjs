// Full-app smoke test: the real app (WebView2, Rust backend) on a synthetic project, driven
// over CDP. Manual, before a release, on a machine with a desktop session; see README.md.
// Fails on any failed check in tools.js, uncaught exception or console error.
//
//   node tools/smoke/app-smoke.mjs [--swiftshader]
import { readFileSync } from "node:fs";
import { launchApp } from "./app.mjs";

const swiftshader = process.argv.includes("--swiftshader")
  ? "--use-angle=swiftshader --enable-unsafe-swiftshader --ignore-gpu-blocklist"
  : "--force_high_performance_gpu";
const problems = [];
let app;
let failed = false;
try {
  app = await launchApp({ webviewArgs: swiftshader });
  const { cdp } = app;
  cdp.on((m) => {
    if (m.method === "Console.messageAdded") {
      const { level, text } = m.params.message;
      if (level === "error" || /shader error|THREE\.WebGLProgram/i.test(text)) problems.push(`${level}: ${text.slice(0, 1500)}`);
    }
    if (m.method === "Runtime.exceptionThrown") problems.push(`exception: ${JSON.stringify(m.params.exceptionDetails).slice(0, 1500)}`);
  });
  await cdp.send("Console.enable"); // replays messages logged before we connected
  await cdp.send("Runtime.enable");
  const summary = await cdp.evaluate(readFileSync(new URL("./tools.js", import.meta.url), "utf8"));
  console.log("checks passed:", JSON.stringify(summary, null, 1));
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
  app?.close();
}
process.exit(failed ? 1 : 0);
