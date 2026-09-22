// Level-of-detail node selection (pure, no three.js): which octree nodes to draw this
// frame, in the order they should load, under a point budget.
//
// Nodes are refined by projected size (larger on screen first). While a measurement tool
// is active, nodes the cursor ray passes through are refined all the way to the leaves
// first, whatever their size, so snapping uses full-resolution points (item 8).

export interface LodNode {
  /** Bounding sphere in render space (project frame minus the render origin). */
  center: [number, number, number];
  radius: number;
  count: number;
  children: number[];
}

export interface LodScan {
  nodes: LodNode[];
  root: number;
}

export interface View {
  eye: [number, number, number];
  /** Six planes (a, b, c, d) with inward normals: a point is inside when ax+by+cz+d >= 0. */
  frustum: [number, number, number, number][];
  /** Pixels per unit of (radius / distance): screenHeight / (2 tan(fov / 2)). */
  projScale: number;
  /** Stop refining nodes smaller than this on screen, px. */
  minNodePx: number;
  budget: number;
  /** Cursor ray: origin, unit direction, and cone half-angle tangent. */
  focus?: { origin: [number, number, number]; dir: [number, number, number]; tan: number };
}

export interface Selection {
  /** [scan, node] pairs, highest priority first. */
  nodes: [number, number][];
  points: number;
  /** True when the budget, not detail, stopped refinement. */
  limited: boolean;
  visited: number;
}

function inFrustum(n: LodNode, planes: View["frustum"]): boolean {
  for (const [a, b, c, d] of planes) {
    if (a * n.center[0] + b * n.center[1] + c * n.center[2] + d < -n.radius) return false;
  }
  return true;
}

/** Does the node's sphere touch the focus cone? */
export function inFocus(n: LodNode, f: NonNullable<View["focus"]>): boolean {
  const v = [n.center[0] - f.origin[0], n.center[1] - f.origin[1], n.center[2] - f.origin[2]];
  const t = v[0] * f.dir[0] + v[1] * f.dir[1] + v[2] * f.dir[2];
  if (t < -n.radius) return false;
  const along = Math.max(t, 0);
  const perp2 = v[0] ** 2 + v[1] ** 2 + v[2] ** 2 - t * t;
  const reach = n.radius + along * f.tan;
  return perp2 <= reach * reach;
}

/** Binary max-heap on priority. */
class Heap {
  private items: { p: number; scan: number; node: number }[] = [];
  push(p: number, scan: number, node: number) {
    const a = this.items;
    a.push({ p, scan, node });
    let i = a.length - 1;
    while (i > 0) {
      const up = (i - 1) >> 1;
      if (a[up].p >= a[i].p) break;
      [a[up], a[i]] = [a[i], a[up]];
      i = up;
    }
  }
  pop() {
    const a = this.items;
    const top = a[0];
    const last = a.pop()!;
    if (a.length > 0) {
      a[0] = last;
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        const r = l + 1;
        let m = i;
        if (l < a.length && a[l].p > a[m].p) m = l;
        if (r < a.length && a[r].p > a[m].p) m = r;
        if (m === i) break;
        [a[m], a[i]] = [a[i], a[m]];
        i = m;
      }
    }
    return top;
  }
  get size() {
    return this.items.length;
  }
}

export function selectNodes(scans: LodScan[], view: View): Selection {
  const heap = new Heap();
  const priority = (n: LodNode) => {
    if (view.focus && inFocus(n, view.focus)) return Infinity;
    const dx = n.center[0] - view.eye[0];
    const dy = n.center[1] - view.eye[1];
    const dz = n.center[2] - view.eye[2];
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz);
    // Inside the sphere: as large as it gets.
    return dist <= n.radius ? 1e9 : (n.radius / dist) * view.projScale;
  };
  scans.forEach((s, si) => {
    const root = s.nodes[s.root];
    if (root && inFrustum(root, view.frustum)) heap.push(priority(root), si, s.root);
  });
  const out: Selection = { nodes: [], points: 0, limited: false, visited: 0 };
  while (heap.size > 0) {
    const { p, scan, node } = heap.pop();
    out.visited++;
    const n = scans[scan].nodes[node];
    if (p !== Infinity && p < view.minNodePx && out.nodes.length > 0) continue;
    if (out.points + n.count > view.budget && p !== Infinity) {
      out.limited = true;
      continue;
    }
    out.nodes.push([scan, node]);
    out.points += n.count;
    for (const c of n.children) {
      const child = scans[scan].nodes[c];
      if (inFrustum(child, view.frustum)) heap.push(priority(child), scan, c);
    }
  }
  return out;
}
