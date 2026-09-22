// In-page functional checks, evaluated in the Locus webview by smoke.mjs (or by hand with
// cdp.mjs). Exercises rendering, picking, every measurement kind, every cleanup kind with
// undo, colour modes and clipping. Returns a summary; throws on any failed expectation.
(async () => {
  const e = globalThis.__locus;
  const inv = window.__TAURI_INTERNALS__.invoke;
  const fail = (msg) => {
    throw new Error(`smoke: ${msg}`);
  };
  if (!e?.layer) fail("no point clouds loaded");
  const out = { gpu: "", scans: e.layer.data.scans.length };
  const gl = e.renderer.getContext();
  const dbg = gl.getExtension("WEBGL_debug_renderer_info");
  out.gpu = String(dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER));

  // Look down into the room so picks land on the floor and crates.
  const o = e.origin;
  const r = (x, y, z) => [x - o[0], y - o[1], z - o[2]];
  e.camera.position.set(...r(20, 4, 14));
  e.controls.target.set(...r(20, 15, 0));
  e.controls.update();
  e.focusActive = true;
  out.render = await e.benchmark(3);
  if (!(out.render.drawn > 0)) fail("nothing drawn");

  // Picks across the view; every one must resolve or be refused, never throw.
  const w = e.host.clientWidth;
  const h = e.host.clientHeight;
  const hits = [];
  for (const [fx, fy] of [[0.35, 0.7], [0.65, 0.7], [0.65, 0.45], [0.35, 0.45], [0.5, 0.3]]) {
    const hit = e.pick(fx * w, fy * h);
    if (hit && !("refused" in hit)) hits.push(hit);
  }
  if (hits.length < 5) fail(`only ${hits.length} of 5 picks hit a point`);
  const resolved = await inv("pick_resolve", { pick: hits[0] });
  if (!Array.isArray(resolved.project)) fail("pick did not resolve");

  // Every measurement kind.
  let st;
  st = await inv("measure", { kind: "distance", picks: hits.slice(0, 2) });
  st = await inv("measure", { kind: "angle", picks: hits.slice(0, 3) });
  st = await inv("measure", { kind: "area", picks: hits.slice(0, 4) });
  st = await inv("measure", { kind: "height", picks: hits.slice(0, 5) });
  out.measurements = st.measurements.slice(-4).map((m) => m.kind);
  for (const m of st.measurements.slice(-4)) {
    const v = m.result.value ?? m.result.area?.value ?? m.result.height?.value;
    const s = m.result.sigma ?? m.result.area?.sigma ?? m.result.height?.sigma;
    if (!Number.isFinite(v) || !(s >= 0)) fail(`${m.kind} has no value with uncertainty`);
  }
  try {
    await inv("pick_resolve", { pick: { ...hits[0], revision: 999 } });
    fail("a stale pick was accepted");
  } catch (err) {
    if (String(err).startsWith("Error: smoke:")) throw err;
  }

  // Lasso: preview both modes, apply the default, and the other cleanups; then undo all.
  const poly = [[w * 0.3, h * 0.3], [w * 0.7, h * 0.3], [w * 0.7, h * 0.7], [w * 0.3, h * 0.7]];
  const visible = await inv("cleanup_preview", { request: e.lassoRequest(poly, "visible_surface") });
  const all = await inv("cleanup_preview", { request: e.lassoRequest(poly, "all_depths") });
  if (!(visible > 0 && all >= visible)) fail(`lasso counts visible ${visible}, all ${all}`);
  out.lasso = { visible, all };
  const before = st.cleanups.length;
  const c = resolved.project;
  const box = { min: c.map((v) => v - 1), max: c.map((v) => v + 1) };
  st = await inv("cleanup_apply", { request: { kind: "box_delete", region: box } });
  st = await inv("cleanup_apply", { request: { kind: "outliers", k: 8, std_mult: 2, region: null } });
  st = await inv("cleanup_apply", { request: { kind: "voxel", size: 0.05, region: null } });
  st = await inv("cleanup_apply", { request: e.lassoRequest(poly, "visible_surface") });
  const applied = st.cleanups.slice(before);
  if (applied.length !== 4) fail(`expected 4 cleanups, got ${applied.length}`);
  for (const c of applied) st = await inv("cleanup_set_active", { id: c.id, active: false });
  if (st.cleanups.slice(before).some((c) => c.active)) fail("undo left an operation active");
  e.setState(st);

  // Colour modes, clipping and EDL off: all must render without shader errors.
  for (const mode of ["intensity", "elevation", "rgb"]) {
    e.setColorMode(mode);
    await e.benchmark(0.5);
  }
  e.setClipBox("inside", "translate");
  e.setClipPlane(true, 2, 1.0, false);
  await e.benchmark(0.5);
  e.setClipBox("off", "translate");
  e.setClipPlane(false, 2, 0, false);
  e.setEdl(false, 1);
  await e.benchmark(0.5);
  e.setEdl(true, 1);
  return out;
})()
