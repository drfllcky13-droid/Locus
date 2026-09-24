// A 360° view: six 90° cube faces around the camera, resampled into an equirectangular image
// (longitude across, latitude down), centred on the view's forward direction.

type V3 = [number, number, number];

const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V3, b: V3): V3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const unit = (a: V3): V3 => {
  const l = Math.hypot(...a);
  return [a[0] / l, a[1] / l, a[2] / l];
};
const neg = (a: V3): V3 => [-a[0], -a[1], -a[2]];

export interface Face {
  /** Looking along `dir` with `up`; image right is dir × up. */
  dir: V3;
  up: V3;
}

/** The camera's frame from its forward direction, levelled (z up): forward, right, up. */
export function basis(forward: V3): { f: V3; r: V3; u: V3 } {
  const f = unit([forward[0], forward[1], 0]);
  const u: V3 = [0, 0, 1];
  return { f, r: cross(f, u), u };
}

/** The six faces around a levelled forward direction. */
export function faces(forward: V3): Face[] {
  const { f, r, u } = basis(forward);
  return [
    { dir: f, up: u },
    { dir: neg(f), up: u },
    { dir: r, up: u },
    { dir: neg(r), up: u },
    { dir: u, up: neg(f) },
    { dir: neg(u), up: f },
  ];
}

/**
 * Where direction `d` falls in a `size`² face image: [face index, x, y] (pixels from the top
 * left), for the face it points into most directly.
 */
export function lookup(fs: Face[], d: V3, size: number): [number, number, number] {
  let k = 0;
  let best = -Infinity;
  fs.forEach((f, i) => {
    const c = dot(d, f.dir);
    if (c > best) [best, k] = [c, i];
  });
  const f = fs[k];
  const right = cross(f.dir, f.up);
  const x = dot(d, right) / best;
  const y = dot(d, f.up) / best;
  return [k, ((x + 1) / 2) * size, ((1 - y) / 2) * size];
}

/** The direction of an equirectangular pixel's centre (longitude 0 is forward, right is +). */
export function direction(forward: V3, px: number, py: number, w: number, h: number): V3 {
  const { f, r, u } = basis(forward);
  const lon = ((px + 0.5) / w - 0.5) * 2 * Math.PI;
  const lat = (0.5 - (py + 0.5) / h) * Math.PI;
  const [cl, sl] = [Math.cos(lat), Math.sin(lat)];
  const [co, so] = [Math.cos(lon), Math.sin(lon)];
  return [0, 1, 2].map((i) => cl * (co * f[i] + so * r[i]) + sl * u[i]) as V3;
}

/** Resample six face images (RGBA, `size`², in `faces` order) into a `w`×`h` equirectangular
 * canvas (nearest pixel). */
export function equirect(
  forward: V3,
  images: ImageData[],
  size: number,
  w: number,
  h: number,
): HTMLCanvasElement {
  const fs = faces(forward);
  const out = document.createElement("canvas");
  [out.width, out.height] = [w, h];
  const ctx = out.getContext("2d")!;
  const img = ctx.createImageData(w, h);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const [k, fx, fy] = lookup(fs, direction(forward, x, y, w, h), size);
      const sx = Math.min(size - 1, Math.max(0, Math.floor(fx)));
      const sy = Math.min(size - 1, Math.max(0, Math.floor(fy)));
      const i = (sy * size + sx) * 4;
      const o = (y * w + x) * 4;
      const src = images[k].data;
      img.data[o] = src[i];
      img.data[o + 1] = src[i + 1];
      img.data[o + 2] = src[i + 2];
      img.data[o + 3] = 255;
    }
  ctx.putImageData(img, 0, 0);
  return out;
}
