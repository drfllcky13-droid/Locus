// The diagram document: what is stored, revision by revision, in the project
// (crates/locus-core/src/diagram.rs). Coordinates are project metres (plan view, x east,
// y north); angles are radians, anticlockwise from +x. Units are converted only for display.

export type Pt = [number, number];

export interface Layer {
  id: string;
  name: string;
  visible: boolean;
  locked: boolean;
  /** CSS colour for the layer's strokes. */
  color: string;
}

interface Base {
  id: string;
  layer: string;
}

/** A hand measurement as taken at the scene, kept with the point it produced. */
export type FieldMeasurement =
  | {
      method: "baseline_offset";
      /** Ids of the two points defining the baseline, from → to. */
      from: string;
      to: string;
      along: number;
      offset: number;
      side: "Left" | "Right";
    }
  | {
      method: "triangulation";
      /** Reference point ids and the taped distance from each. */
      refs: { point: string; distance: number }[];
      side: "Left" | "Right" | null;
    };

export type Entity =
  | (Base & { kind: "line"; a: Pt; b: Pt })
  | (Base & { kind: "polyline"; points: Pt[]; closed: boolean })
  /** Anticlockwise from `start` to `end`. */
  | (Base & { kind: "arc"; center: Pt; radius: number; start: number; end: number })
  /** Aligned dimension between `a` and `b`, drawn `offset` metres to the left of a→b. */
  | (Base & { kind: "dimension"; a: Pt; b: Pt; offset: number })
  /** `height` is the printed text height in millimetres on paper. */
  | (Base & { kind: "text"; at: Pt; text: string; height: number; rotation: number })
  | (Base & { kind: "symbol"; symbol: string; at: Pt; rotation: number; scale: number })
  | (Base & { kind: "marker"; number: number; at: Pt; note: string })
  /**
   * A reference or measured point. `measurement` records how a measured point was taken;
   * `sigma` is its solved 1σ radius (m), from the hand-measurement solver.
   */
  | (Base & {
      kind: "point";
      at: Pt;
      label: string;
      measurement: FieldMeasurement | null;
      sigma: number | null;
    })
  | (Base & { kind: "north"; at: Pt; rotation: number })
  /** A scale bar `length` metres long. */
  | (Base & { kind: "scalebar"; at: Pt; length: number })
  | (Base & { kind: "legend"; at: Pt });

export type EntityKind = Entity["kind"];

export interface Diagram {
  version: 1;
  layers: Layer[];
  entities: Entity[];
}

export function emptyDiagram(): Diagram {
  return {
    version: 1,
    layers: [{ id: "base", name: "Base", visible: true, locked: false, color: "#e6e8eb" }],
    entities: [],
  };
}
