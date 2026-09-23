import { describe, expect, it } from "vitest";
import { emptyDiagram } from "./model";
import { commit, gridStep, history, pan, redo, toScreen, toWorld, undo, zoomAt } from "./view";

const v = { center: [10, 5] as [number, number], scale: 20, width: 800, height: 600 };

describe("diagram view", () => {
  it("maps world to screen with y up and back", () => {
    expect(toScreen(v, [10, 5])).toEqual([400, 300]);
    expect(toScreen(v, [11, 6])).toEqual([420, 280]);
    const w = toWorld(v, [123, 456]);
    const s = toScreen(v, w);
    expect(s[0]).toBeCloseTo(123, 9);
    expect(s[1]).toBeCloseTo(456, 9);
  });

  it("zooms about the cursor and pans with the drag", () => {
    const at: [number, number] = [600, 100];
    const before = toWorld(v, at);
    const z = zoomAt(v, at, 2);
    const after = toWorld(z, at);
    expect(z.scale).toBe(40);
    expect(after[0]).toBeCloseTo(before[0], 9);
    expect(after[1]).toBeCloseTo(before[1], 9);
    // Dragging right by 20 px moves the view's centre 1 m left (at 20 px/m).
    expect(pan(v, 20, 0).center).toEqual([9, 5]);
  });

  it("picks round grid steps", () => {
    expect(gridStep(20)).toBe(1);
    expect(gridStep(200)).toBe(0.1);
    expect(gridStep(7)).toBe(5);
  });

  it("undoes and redoes whole documents", () => {
    const a = emptyDiagram();
    const b = {
      ...a,
      entities: [
        {
          id: "1",
          layer: "base",
          kind: "north" as const,
          at: [0, 0] as [number, number],
          rotation: 0,
        },
      ],
    };
    let h = commit(history(a), b);
    expect(h.present).toBe(b);
    h = undo(h);
    expect(h.present).toBe(a);
    h = redo(h);
    expect(h.present).toBe(b);
    expect(undo(history(a)).present).toBe(a);
  });
});
