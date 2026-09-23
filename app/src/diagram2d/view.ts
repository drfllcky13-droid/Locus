// Screen ↔ world mapping for the diagram canvas, and the editor's undo history. Pure.
import type { Diagram, Pt } from "./model";

/** World point at the screen centre, and pixels per metre. World y is up, screen y down. */
export interface View {
  center: Pt;
  scale: number;
  width: number;
  height: number;
}

export function toScreen(v: View, p: Pt): Pt {
  return [
    v.width / 2 + (p[0] - v.center[0]) * v.scale,
    v.height / 2 - (p[1] - v.center[1]) * v.scale,
  ];
}

export function toWorld(v: View, s: Pt): Pt {
  return [
    v.center[0] + (s[0] - v.width / 2) / v.scale,
    v.center[1] - (s[1] - v.height / 2) / v.scale,
  ];
}

/** Zoom by `factor` keeping the world point under screen point `s` fixed. */
export function zoomAt(v: View, s: Pt, factor: number): View {
  const w = toWorld(v, s);
  const scale = Math.min(Math.max(v.scale * factor, 0.5), 50_000);
  const next = { ...v, scale };
  const w2 = toWorld(next, s);
  return { ...next, center: [v.center[0] + w[0] - w2[0], v.center[1] + w[1] - w2[1]] };
}

/** Pan by a screen-pixel drag. */
export function pan(v: View, dx: number, dy: number): View {
  return { ...v, center: [v.center[0] - dx / v.scale, v.center[1] + dy / v.scale] };
}

/** A grid spacing that gives lines 20–100 px apart at this zoom (1, 2 or 5 × 10ⁿ m). */
export function gridStep(scale: number): number {
  const target = 20 / scale;
  const p = 10 ** Math.floor(Math.log10(target));
  for (const m of [1, 2, 5, 10]) if (m * p >= target) return m * p;
  return 10 * p;
}

/** Undo history of whole documents (diagrams are small; snapshots keep it simple). */
export interface History {
  past: Diagram[];
  present: Diagram;
  future: Diagram[];
}

export const history = (d: Diagram): History => ({ past: [], present: d, future: [] });

export function commit(h: History, next: Diagram): History {
  // ponytail: whole-document snapshots, capped at 200; diffs if diagrams grow huge
  return { past: [...h.past, h.present].slice(-200), present: next, future: [] };
}

export function undo(h: History): History {
  if (!h.past.length) return h;
  return {
    past: h.past.slice(0, -1),
    present: h.past[h.past.length - 1],
    future: [h.present, ...h.future],
  };
}

export function redo(h: History): History {
  if (!h.future.length) return h;
  return { past: [...h.past, h.present], present: h.future[0], future: h.future.slice(1) };
}
