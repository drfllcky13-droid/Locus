// Print console messages and exceptions buffered in the Locus webview.
//   node tools/smoke/cdplog.mjs [port]
import { connect } from "./cdp.mjs";

const c = await connect(Number(process.argv[2] ?? 9222), 5_000);
c.on((m) => {
  if (m.method === "Console.messageAdded") console.log(m.params.message.level, m.params.message.text.slice(0, 2000));
  if (m.method === "Runtime.exceptionThrown") console.log("exception", JSON.stringify(m.params.exceptionDetails).slice(0, 1000));
});
await c.send("Console.enable");
await c.send("Runtime.enable");
setTimeout(() => c.close(), 1500);
