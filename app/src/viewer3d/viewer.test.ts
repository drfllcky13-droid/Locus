import { describe, expect, test } from "vitest";
import { BUDGET_MAX, BUDGET_MIN, BUDGET_START, initialBudget, updateBudget } from "./budget";
import { dataBounds, inFocus, selectNodes, type LodNode, type LodScan, type View } from "./lod";
import {
  MAX_POINTS_PER_NODE,
  MAX_SLOTS,
  UNPICKABLE_SLOT,
  decode,
  encode,
  nearest,
  nearestSurface,
  slotFor,
} from "./pick";

describe("pick encoding", () => {
  test("round-trips slot and index", () => {
    for (const [slot, index] of [
      [1, 0],
      [4095, 1_048_575],
      [37, 123_456],
    ]) {
      expect(decode(...encode(slot, index))).toEqual({ slot, index });
    }
    expect(decode(0, 0, 0, 0)).toBeNull();
  });

  test("nearest hit to the window centre wins", () => {
    const size = 5;
    const px = new Uint8Array(size * size * 4);
    const put = (x: number, y: number, slot: number, index: number) =>
      px.set(encode(slot, index), (y * size + x) * 4);
    put(0, 0, 1, 10);
    put(3, 2, 2, 20); // one pixel right of centre
    put(4, 4, 3, 30);
    expect(nearest(px, size)).toEqual({ kind: "hit", slot: 2, index: 20 });
    expect(nearest(new Uint8Array(size * size * 4), size)).toBeNull();
  });
});

describe("picking the nearest surface", () => {
  const size = 7;
  const put = (px: Uint8Array, x: number, y: number, slot: number, index: number) =>
    px.set(encode(slot, index), (y * size + x) * 4);

  test("a farther surface seen between a near surface's points loses", () => {
    // Close up: the near surface's points (slot 1, 0.4 m away) sit on a sparse grid; through
    // the gap at the centre a far wall point (slot 2, 1.9 m) is drawn.
    const px = new Uint8Array(size * size * 4);
    put(px, 3, 3, 2, 99); // far, at the centre
    put(px, 1, 1, 1, 10);
    put(px, 5, 1, 1, 11);
    put(px, 1, 5, 1, 12);
    put(px, 5, 4, 1, 13); // near, nearest the centre among the near ones
    const depth = (slot: number) => (slot === 1 ? 0.4 : 1.9);
    expect(nearest(px, size)).toEqual({ kind: "hit", slot: 2, index: 99 });
    expect(nearestSurface(px, size, (s) => depth(s))).toEqual({ kind: "hit", slot: 1, index: 13 });
  });

  test("regression: a click between a close surface's points doesn't reach the wall behind", () => {
    // As found in the app: a panel 0.4 m away scanned at 5 mm spacing is a 12 px grid of
    // (at most) 8 px sprites; a wall 1.9 m away shows through the 4 px gaps. The click lands
    // in a gap, so the pixel nearest the cursor is the wall's.
    const size = 25; // the 12 px pick radius
    const px = new Uint8Array(size * size * 4);
    const put = (x: number, y: number, slot: number, index: number) =>
      px.set(encode(slot, index), (y * size + x) * 4);
    // The wall first (dense, everywhere), then the panel's sprites drawn over it.
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) put(x, y, 2, y * size + x);
    let k = 0;
    for (let gy = -12; gy <= 24; gy += 12)
      for (let gx = -12; gx <= 24; gx += 12) {
        k++;
        for (let y = gy + 2; y < gy + 10; y++)
          for (let x = gx + 2; x < gx + 10; x++)
            if (x >= 0 && y >= 0 && x < size && y < size) put(x, y, 1, k);
      }
    const c = (size - 1) / 2; // the centre pixel (12, 12) is in a gap
    expect(nearest(px, size)).toMatchObject({ slot: 2 });
    const pick = nearestSurface(px, size, (slot) => (slot === 1 ? 0.4 : 1.9));
    expect(pick).toMatchObject({ kind: "hit", slot: 1 });
    expect(c).toBe(12);
  });

  test("points on the same surface are chosen by distance from the cursor", () => {
    const px = new Uint8Array(size * size * 4);
    put(px, 3, 4, 1, 1); // 1 px from the centre, 1.000 m away
    put(px, 0, 0, 1, 2); // corner, 0.995 m away: nearer, but the same surface
    const depths: Record<number, number> = { 1: 1.0, 2: 0.995 };
    expect(nearestSurface(px, size, (_, i) => depths[i])).toEqual({
      kind: "hit",
      slot: 1,
      index: 1,
    });
  });

  test("the unpickable rule still applies", () => {
    const px = new Uint8Array(size * size * 4);
    put(px, 3, 3, UNPICKABLE_SLOT, 0);
    put(px, 0, 0, 1, 5);
    expect(nearestSurface(px, size, () => 1)?.kind).toBe("refused");
  });
});

describe("pick limits are refused, never guessed", () => {
  test("a node past the slot limit is unpickable, and picking it is refused", () => {
    // Fill every encodable slot, as with thousands of nodes loaded.
    const slots: (string | null)[] = [null];
    for (let i = 1; i <= MAX_SLOTS; i++) {
      const s = slotFor(10, slots);
      expect(s).toBe(i);
      slots.push(`scan/${i}`);
    }
    const overflow = slotFor(10, slots);
    expect(overflow).toBe(UNPICKABLE_SLOT);

    // The overflow node is drawn nearest the cursor; a pickable node is further out.
    const size = 5;
    const px = new Uint8Array(size * size * 4);
    px.set(encode(overflow, 7), (2 * size + 2) * 4);
    px.set(encode(3, 9), (0 * size + 0) * 4);
    const r = nearest(px, size);
    expect(r?.kind).toBe("refused");
  });

  test("a node with more points than 20 bits can index is unpickable", () => {
    expect(slotFor(MAX_POINTS_PER_NODE, [null])).toBe(1);
    expect(slotFor(MAX_POINTS_PER_NODE + 1, [null])).toBe(UNPICKABLE_SLOT);
  });

  test("freed slots are reused before the limit is reached", () => {
    const slots = [null, "a", null, "c"];
    expect(slotFor(10, slots)).toBe(2);
  });
});

describe("adaptive budget", () => {
  test("shrinks on slow frames and grows only when limited and fast", () => {
    let s = initialBudget();
    expect(s.points).toBe(BUDGET_START);
    for (let i = 0; i < 200; i++) s = updateBudget(s, 60, true); // integrated-GPU speed
    expect(s.points).toBe(BUDGET_MIN);
    let f = initialBudget();
    for (let i = 0; i < 200; i++) f = updateBudget(f, 8, false);
    expect(f.points).toBe(BUDGET_START); // fast but not limited: no reason to grow
    for (let i = 0; i < 400; i++) f = updateBudget(f, 8, true);
    expect(f.points).toBe(BUDGET_MAX);
  });
});

// A quadtree-like hierarchy: root with 4 children, each with 4 leaves, along the x axis.
function tree(): LodScan {
  const nodes: LodNode[] = [{ center: [0, 0, 0], radius: 8, count: 100, children: [] }];
  for (let i = 0; i < 4; i++) {
    const c = nodes.length;
    nodes.push({ center: [-6 + i * 4, 0, 0], radius: 4, count: 100, children: [] });
    nodes[0].children.push(c);
    for (let j = 0; j < 4; j++) {
      nodes.push({ center: [-7 + i * 4 + j, 0, 0], radius: 1, count: 100, children: [] });
      nodes[c].children.push(nodes.length - 1);
    }
  }
  return { nodes, root: 0 };
}

const everything: View["frustum"] = [[1, 0, 0, 1e9]];

function view(over: Partial<View> = {}): View {
  return {
    eye: [0, -40, 0],
    frustum: everything,
    projScale: 2000,
    minNodePx: 30,
    budget: 1e9,
    ...over,
  };
}

describe("LOD selection", () => {
  test("larger on screen loads first, and detail stops at the minimum size", () => {
    const s = selectNodes([tree()], view());
    expect(s.nodes[0]).toEqual([0, 0]);
    expect(s.nodes.length).toBe(21); // root, 4 children, 16 leaves: all at least 50 px from 40 m
    const far = selectNodes([tree()], view({ eye: [0, -200, 0] }));
    expect(far.nodes.length).toBe(5); // leaves are 10 px from 200 m
  });

  test("the budget limits the selection", () => {
    const s = selectNodes([tree()], view({ budget: 550 }));
    expect(s.points).toBeLessThanOrEqual(550);
    expect(s.limited).toBe(true);
  });

  test("frustum culling drops nodes behind a plane", () => {
    // Keep only x >= 0.
    const s = selectNodes([tree()], view({ frustum: [[1, 0, 0, 0]] }));
    expect(s.nodes.every(([, n]) => tree().nodes[n].center[0] + tree().nodes[n].radius >= 0)).toBe(
      true,
    );
  });

  test("the cursor ray is refined to the leaves first, even when far away", () => {
    // From 300 m the leaves would normally be skipped; the ray passes through x = 0.5.
    const focus = {
      origin: [0.5, -300, 0] as [number, number, number],
      dir: [0, 1, 0] as [number, number, number],
      tan: 0.0005,
    };
    const s = selectNodes([tree()], view({ eye: [0, -300, 0], focus, budget: 1e9 }));
    const t = tree();
    const leavesOnRay = s.nodes.filter(([, n]) => t.nodes[n].radius === 1);
    expect(leavesOnRay.length).toBeGreaterThan(0);
    expect(leavesOnRay.every(([, n]) => inFocus(t.nodes[n], focus))).toBe(true);
    // Focus nodes come before everything else.
    const firstNonFocus = s.nodes.findIndex(([, n]) => !inFocus(t.nodes[n], focus));
    expect(s.nodes.slice(0, firstNonFocus).length).toBeGreaterThanOrEqual(3);
  });
});

describe("data bounds", () => {
  // A 20 m cube holding a room 20 m × 16 m × 3 m on its floor: nodes three levels down are 2.5 m.
  const node = (name: string, min: number[], size: number, count = 1) => ({
    name,
    min: min as [number, number, number],
    size,
    count,
    spacing: 0,
    children: 0,
  });
  const nodes = [node("r", [0, 0, 0], 20)];
  for (let x = 0; x < 8; x++)
    for (let y = 0; y < 8; y++) {
      if (y * 2.5 >= 16) continue;
      for (let z = 0; z < 2; z++)
        nodes.push(node(`r${x}${y}${z}`, [x * 2.5, y * 2.5, z * 2.5], 2.5));
    }
  nodes.push(node("r777", [17.5, 17.5, 17.5], 2.5, 0)); // an empty node doesn't count
  const shifted = [1, 0, 0, 100, 0, 1, 0, 200, 0, 0, 1, 0, 0, 0, 0, 1];

  test("hug the occupied nodes, not the root cube, in render space", () => {
    const b = dataBounds([{ pose: shifted, nodes }], [100, 200, 0])!;
    expect(b.min).toEqual([0, 0, 0]);
    expect(b.max).toEqual([20, 17.5, 5]);
  });

  test("a single-node cloud falls back to its root cube; no scans, no bounds", () => {
    const b = dataBounds([{ pose: shifted, nodes: [nodes[0]] }], [0, 0, 0])!;
    expect(b.min).toEqual([100, 200, 0]);
    expect(b.max).toEqual([120, 220, 20]);
    expect(dataBounds([], [0, 0, 0])).toBeNull();
  });
});
