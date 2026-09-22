(async () => {
  const e = __locus;
  const o = e.origin;
  const r = (x, y, z) => [x - o[0], y - o[1], z - o[2]];
  const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
  const views = {
    overview: () => { e.frame(e.layer.data); },
    interior: () => { e.camera.position.set(...r(8, 5, 1.7)); e.controls.target.set(...r(20, 15, 1.3)); e.controls.update(); },
  };
  const out = { canvas: [e.renderer.domElement.width, e.renderer.domElement.height],
    gpu: (() => { const gl = e.renderer.getContext(); return gl.getParameter(gl.getExtension('WEBGL_debug_renderer_info').UNMASKED_RENDERER_WEBGL); })(),
    scans: e.layer.data.scans.length,
    totalNodes: e.layer.data.scans.reduce((n, s) => n + s.nodes.length, 0),
    totalPoints: e.layer.data.scans.reduce((n, s) => n + s.points, 0) };
  for (const [name, set] of Object.entries(views)) {
    set(); e.requestRender();
    await e.benchmark(10);           // warm-up: loads nodes, settles the adaptive budget
    out[name] = await e.benchmark(20);
    await sleep(500);
  }
  return out;
})()
