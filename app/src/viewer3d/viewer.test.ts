import { describe, expect, test } from "vitest";
import { BUDGET_MAX, BUDGET_MIN, BUDGET_START, initialBudget, updateBudget } from "./budget";
import { inFocus, selectNodes, type LodNode, type LodScan, type View } from "./lod";
import {
  MAX_POINTS_PER_NODE,
  MAX_SLOTS,
  UNPICKABLE_SLOT,
  decode,
  encode,
  nearest,
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
