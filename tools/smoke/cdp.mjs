// Minimal Chrome DevTools Protocol client for the Lotus webview (WebView2 started with
// --remote-debugging-port). Used by smoke.mjs; also runnable by hand:
//   node tools/smoke/cdp.mjs "<expression>" [port]      evaluate and print the result
export async function connect(port = 9222, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  let page;
  while (!page) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      page = list.find((t) => t.type === "page");
    } catch {
      // not up yet
    }
    if (!page) {
      if (Date.now() > deadline) throw new Error(`no webview on port ${port}`);
      await new Promise((r) => setTimeout(r, 500));
    }
  }
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => {
    ws.addEventListener("open", res, { once: true });
    ws.addEventListener("error", rej, { once: true });
  });
  let id = 0;
  const waiting = new Map();
  const listeners = [];
  ws.addEventListener("message", (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && waiting.has(m.id)) {
      waiting.get(m.id)(m);
      waiting.delete(m.id);
    } else if (m.method) listeners.forEach((l) => l(m));
  });
  const send = (method, params = {}) =>
    new Promise((res) => {
      const my = ++id;
      waiting.set(my, res);
      ws.send(JSON.stringify({ id: my, method, params }));
    });
  return {
    send,
    on: (f) => listeners.push(f),
    /** Evaluate an expression (awaiting promises) and return its value, or throw. */
    async evaluate(expression) {
      const m = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (m.result?.exceptionDetails) {
        const d = m.result.exceptionDetails;
        // Tauri commands reject with plain strings, which carry a value but no description.
        const ex = d.exception;
        throw new Error(ex?.description ?? (ex && "value" in ex ? JSON.stringify(ex.value) : d.text));
      }
      return m.result?.result?.value;
    },
    close: () => ws.close(),
  };
}

if (process.argv[1]?.endsWith("cdp.mjs")) {
  const [expr, port] = process.argv.slice(2);
  if (expr) {
    const c = await connect(Number(port ?? 9222), 5_000);
    try {
      console.log(JSON.stringify(await c.evaluate(expr), null, 1));
    } finally {
      c.close();
    }
  }
}
