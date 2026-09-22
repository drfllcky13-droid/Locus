// GPU pick encoding. The pick pass only identifies a point; its coordinates are always
// resolved in Rust from the stored f64 data (see docs/methods/measurement.md).
//
// Each pixel is RGBA8: 12 bits of node slot (1-based, 0 = no point) and 20 bits of the
// point's position within the node as served.

export const MAX_SLOTS = 4095;
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

/** Nearest encoded point to the centre of a square pick window (row-major RGBA pixels). */
export function nearest(pixels: Uint8Array, size: number): { slot: number; index: number } | null {
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
  return best;
}

/** GLSL for the pick fragment: same packing as `encode`. */
export const PICK_GLSL = `
vec4 encodePick(float slot, float index) {
  float hi = slot * 16.0 + floor(index / 65536.0);
  float mid = mod(floor(index / 256.0), 256.0);
  float lo = mod(index, 256.0);
  return vec4(floor(hi / 256.0), mod(hi, 256.0), mid, lo) / 255.0;
}`;
