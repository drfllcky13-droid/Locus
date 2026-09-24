// What a rendered frame carries on top of the scene. Overlays are optional and off by default;
// `labels` (the driver-view limitation) are drawn whatever the overlays. See
// docs/methods/animation.md.

export interface Overlays {
  elapsed_time: boolean;
  frame_number: boolean;
  speeds: boolean;
  scale_bar: boolean;
  illustrative: boolean;
  time_zero: boolean;
}

export const NO_OVERLAYS: Overlays = {
  elapsed_time: false,
  frame_number: false,
  speeds: false,
  scale_bar: false,
  illustrative: false,
  time_zero: false,
};

export interface FrameInfo {
  t: number;
  frame: number;
  frames: number;
  timeZero: string;
  view: string;
  hfovDeg: number;
  /** Name, speed (m/s), and whether its motion is assumed now. */
  movers: [string, number, boolean][];
  /** Distance from the camera to what it looks at (m), for the scale bar. */
  targetDistance: number;
  labels: string[];
}

/**
 * A round scale-bar length (m) and its length in pixels, true at `distance` from the camera:
 * the bar only holds at that depth in a perspective view, and says so.
 */
export function scaleBar(width: number, height: number, hfovDeg: number, distance: number) {
  const aspect = width / height;
  const tanV = Math.tan((hfovDeg * Math.PI) / 360) / aspect;
  const pxPerM = height / 2 / (tanV * distance);
  const lengths = [0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100];
  const m = lengths.filter((l) => l * pxPerM <= width / 4).pop() ?? lengths[0];
  return { metres: m, px: m * pxPerM };
}

export function drawOverlays(c: HTMLCanvasElement, o: Overlays, f: FrameInfo) {
  const ctx = c.getContext("2d")!;
  // Upright, whatever the canvas was drawn with (a mirror view's image is flipped).
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  const { width: w, height: h } = c;
  const size = Math.max(12, Math.round(h / 36));
  const pad = Math.round(size * 0.6);
  ctx.font = `${size}px system-ui, sans-serif`;
  ctx.textBaseline = "top";
  const box = (lines: string[], x: number, y: number, right = false, colour = "#ffffff") => {
    if (!lines.length) return;
    const tw = Math.max(...lines.map((l) => ctx.measureText(l).width));
    const bh = lines.length * size * 1.3 + pad;
    const bx = right ? x - tw - 2 * pad : x;
    const by = y < 0 ? h + y - bh : y;
    ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
    ctx.fillRect(bx, by, tw + 2 * pad, bh);
    ctx.fillStyle = colour;
    lines.forEach((l, i) => ctx.fillText(l, bx + pad, by + pad / 2 + i * size * 1.3));
  };
  const any = Object.values(o).some(Boolean);
  const tl: string[] = [];
  if (o.elapsed_time)
    tl.push(`t = ${f.t < 0 ? "−" : f.t > 0 ? "+" : ""}${Math.abs(f.t).toFixed(2)} s`);
  if (o.time_zero) tl.push(`Time zero: ${f.timeZero}`);
  if (o.frame_number) tl.push(`Frame ${f.frame + 1} of ${f.frames}`);
  if (any) tl.push(`${f.view}, ${f.hfovDeg.toFixed(0)}° horizontal`);
  box(tl, pad, pad);
  if (o.speeds)
    box(
      f.movers.map(([n, v]) => `${n}: ${(v * 3.6).toFixed(1)} km/h`),
      w - pad,
      pad,
      true,
    );
  const assumed = f.movers.filter(([, , a]) => a).map(([n]) => n);
  if (o.illustrative && assumed.length)
    box([`Illustrative: assumed motion (${assumed.join(", ")})`], w - pad, -pad, true, "#f2a53a");
  box(f.labels, pad, -pad);
  // A scale bar means nothing across a 360° image.
  if (o.scale_bar && f.targetDistance > 0 && f.hfovDeg < 180) {
    const s = scaleBar(w, h, f.hfovDeg, f.targetDistance);
    const y = h - pad - size * 4;
    const x = w / 2 - s.px / 2;
    ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
    ctx.fillRect(x - pad, y - size * 1.6, s.px + 2 * pad, size * 2.6);
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(x, y, s.px, Math.max(2, size / 5));
    const label = `${s.metres} m at ${f.targetDistance.toFixed(1)} m from the camera`;
    ctx.fillText(label, w / 2 - ctx.measureText(label).width / 2, y - size * 1.4);
  }
}
