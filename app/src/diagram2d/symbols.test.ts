import { describe, expect, it } from "vitest";
import { emptyDiagram, type Diagram, type Entity } from "./model";
import { legendItems, nextMarker, renumberMarkers, SYMBOLS, symbolById } from "./symbols";

const marker = (id: string, number: number, x: number, y: number): Entity => ({
  id,
  layer: "base",
  kind: "marker",
  number,
  at: [x, y],
  note: "",
});
const sym = (id: string, symbol: string, layer = "base"): Entity => ({
  id,
  layer,
  kind: "symbol",
  symbol,
  at: [0, 0],
  rotation: 0,
  scale: 1,
});

describe("symbol library", () => {
  it("loads every symbol with its drawing", () => {
    expect(SYMBOLS.length).toBeGreaterThanOrEqual(15);
    for (const s of SYMBOLS) {
      expect(s.body.length, s.id).toBeGreaterThan(20);
      expect(s.body).not.toContain("<svg");
      expect(s.size).toBeGreaterThan(0);
    }
    expect(symbolById.get("car")?.name).toBe("Car");
  });
});

describe("legend", () => {
  it("lists what is used and updates as the diagram changes", () => {
    let d: Diagram = emptyDiagram();
    expect(legendItems(d)).toEqual([]);
    d = { ...d, entities: [sym("1", "blood"), sym("2", "car"), sym("3", "blood")] };
    // Library order (car before blood), with counts.
    expect(legendItems(d).map((i) => [i.label, i.count])).toEqual([
      ["Car", 1],
      ["Blood", 2],
    ]);
    d = {
      ...d,
      entities: [
        ...d.entities,
        marker("m1", 1, 0, 0),
        marker("m2", 2, 1, 0),
        marker("m5", 5, 2, 0),
      ],
    };
    expect(legendItems(d).map((i) => i.label)).toContain("Evidence markers 1–2, 5");
    // Removing the car removes it from the legend.
    d = { ...d, entities: d.entities.filter((e) => e.id !== "2") };
    expect(legendItems(d).map((i) => i.label)).not.toContain("Car");
  });

  it("leaves out hidden layers and tells measured points from reference points", () => {
    const d: Diagram = {
      ...emptyDiagram(),
      layers: [
        { id: "base", name: "Base", visible: true, locked: false, color: "#fff" },
        { id: "off", name: "Hidden", visible: false, locked: false, color: "#fff" },
      ],
      entities: [
        sym("1", "knife", "off"),
        {
          id: "p",
          layer: "base",
          kind: "point",
          at: [0, 0],
          label: "A",
          measurement: null,
          sigma: null,
        },
        {
          id: "q",
          layer: "base",
          kind: "point",
          at: [1, 1],
          label: "B",
          measurement: { method: "triangulation", refs: [], side: null },
          sigma: 0.002,
        },
      ],
    };
    expect(legendItems(d).map((i) => i.label)).toEqual([
      "Reference point",
      "Measured point (tape)",
    ]);
  });
});

describe("evidence markers", () => {
  it("numbers new markers after the highest, and renumbers in reading order", () => {
    const d: Diagram = {
      ...emptyDiagram(),
      entities: [
        marker("a", 7, 5, 0),
        marker("b", 2, 0, 10),
        marker("c", 3, 3, 10.2),
        marker("d", 9, 0, 0),
      ],
    };
    expect(nextMarker(d)).toBe(10);
    const r = renumberMarkers(d);
    const num = (id: string) => (r.entities.find((e) => e.id === id) as { number: number }).number;
    // Top row (y ≈ 10): b then c; bottom row (y = 0): d then a.
    expect([num("b"), num("c"), num("d"), num("a")]).toEqual([1, 2, 3, 4]);
  });
});
