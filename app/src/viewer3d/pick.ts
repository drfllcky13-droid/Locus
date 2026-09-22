// GPU pick encoding. The pick pass only identifies a point; its coordinates are always
// resolved in Rust from the stored f64 data (see docs/methods/measurement.md).
//
// Each pixel is RGBA8: 12 bits of node slot and 20 bits of the point's position within
// the node as served. Slot 0 means no point. The top slot is reserved: it marks a point
// that was drawn but cannot be identified safely (too many nodes loaded, or an index past
// 20 bits), so a pick landing on it is refused instead of risking a wrong point.

/** Slots 1..MAX_SLOTS identify nodes. */
export const MAX_SLOTS = 4094;
export const UNPICKABLE_SLOT = 4095;
export const MAX_POINTS_PER_NODE = 1 << 20;

export function encode(slot: number, index: number): [number, number, number, number] {
  const v = ((slot & 0xfff) << 20) | (index & 0xfffff);
  return [(v >>> 24) & 255, (v >>> 16) & 255, (v >>> 8) & 255, v & 255];
}

export function decode(
  r: number,
  g: number,
  b: number,
  a: number,
): { slot: number; index: number } | null {
  const v = ((r << 24) | (g << 16) | (b << 8) | a) >>> 0;
  const slot = v >>> 20;
  return slot === 0 ? null : { slot, index: v & 0xfffff };
}

/** The slot a newly loaded node gets: a free one if it can be encoded, else unpickable. */
export function slotFor(points: number, slots: (string | null)[]): number {
  if (points > MAX_POINTS_PER_NODE) return UNPICKABLE_SLOT;
  const free = slots.indexOf(null, 1);
  if (free > 0) return free;
  return slots.length <= MAX_SLOTS ? slots.length : UNPICKABLE_SLOT;
}

export type PickResult =
  { kind: "hit"; slot: number; index: number } | { kind: "refused"; reason: string } | null;

export const REFUSED_REASON =
  "This point can't be identified reliably: too much of the cloud is loaded for the picker " +
  "to tell points apart here. Zoom in closer, or lower the point budget, and pick again.";

/**
 * The encoded point nearest the centre of a square pick window (row-major RGBA pixels).
 * If the nearest drawn point is unpickable the pick is refused, never passed to a neighbour.
 */
export function nearest(pixels: Uint8Array, size: number): PickResult {
  const c = (size - 1) / 2;
  let best: { slot: number; index: number } | null = null;
  let bestD = Infinity;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      const hit = decode(pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]);
      const d = (x - c) ** 2 + (y - c) ** 2;
      if (hit && d < bestD) {
        best = hit;
        bestD = d;
      }
    }
  }
  if (!best) return null;
  if (best.slot === UNPICKABLE_SLOT) return { kind: "refused", reason: REFUSED_REASON };
  return { kind: "hit", ...best };
}

/** GLSL for the pick fragment: same packing as `encode`; indices past 20 bits are unpickable. */
export const PICK_GLSL = `
vec4 encodePick(float slot, float index) {
  if (index >= ${MAX_POINTS_PER_NODE}.0) { slot = ${UNPICKABLE_SLOT}.0; index = 0.0; }
  float hi = slot * 16.0 + floor(index / 65536.0);
  float mid = mod(floor(index / 256.0), 256.0);
  float lo = mod(index, 256.0);
  return vec4(floor(hi / 256.0), mod(hi, 256.0), mid, lo) / 255.0;
}`;
