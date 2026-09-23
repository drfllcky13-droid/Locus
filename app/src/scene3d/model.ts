// The 3D scene document: what is stored, revision by revision, in the project
// (crates/locus-core/src/document.rs, schema 5). Project frame: metres, right-handed, z up,
// x east and y north as in the diagrams. Transforms are f64; the view converts them to
// f32 relative to its render origin.
import type { Part, ExtrudeParams } from "./extrude";
import type { Asset, Slot } from "./library";
import type { RoofParams } from "./roof";

/** A diagram revision something was built from (id, revision id and hash). */
export interface DiagramRef {
  id: number;
  revision: number;
  sha256: string;
  name: string;
}

export interface MaterialDef {
  preset: string;
  /** sRGB hex. */
  color: string;
  roughness: number;
  metalness: number;
  opacity: number;
}

/** Where a model was snapped to the point cloud, as the surface fit reported it. */
export interface SnapRecord {
  point: [number, number, number];
  normal: [number, number, number];
  rms: number;
  max_abs: number;
  points: number;
  radius: number;
  aligned: boolean;
}

export type LightDef =
  | {
      type: "point";
      position: [number, number, number];
      color: string;
      intensity: number;
      range: number;
    }
  | {
      type: "spot";
      position: [number, number, number];
      target: [number, number, number];
      color: string;
      intensity: number;
      /** Half-angle of the cone, degrees. */
      angle: number;
    }
  | { type: "directional"; direction: [number, number, number]; color: string; intensity: number }
  | { type: "ambient"; color: string; intensity: number };

interface Base {
  id: string;
  name: string;
  visible: boolean;
}

export type SceneObject =
  | (Base & {
      kind: "extrusion";
      diagram: DiagramRef;
      params: ExtrudeParams;
      materials: Partial<Record<Part, MaterialDef>>;
    })
  | (Base & {
      kind: "roof";
      diagram: DiagramRef;
      /** The room entity in that diagram revision. */
      room: string;
      params: RoofParams;
      material: MaterialDef;
    })
  | (Base & {
      kind: "model";
      asset: Asset;
      /** 4×4, column-major, model to project frame. */
      matrix: number[];
      materials: Partial<Record<Slot, MaterialDef>>;
      snap: SnapRecord | null;
    })
  | (Base & { kind: "light"; light: LightDef });

export interface Sun {
  on: boolean;
  /** Degrees; north and east positive. */
  lat: number;
  lon: number;
  /** UTC, ISO 8601. */
  time: string;
  /** Direction of true north in the project frame: degrees anticlockwise from +y. */
  north: number;
  intensity: number;
  shadows: boolean;
  /** The position computed for this place and time (locus-analysis sun::sun_position). */
  computed: { azimuth: number; apparent_elevation: number; uncertainty: number } | null;
}

export interface SceneDoc {
  version: 1;
  objects: SceneObject[];
  sun: Sun;
  /** Soft fill so unlit sides aren't black (0 for none). */
  ambient: number;
}

export const EMPTY_SCENE: SceneDoc = {
  version: 1,
  objects: [],
  sun: {
    on: false,
    lat: 0,
    lon: 0,
    time: "2026-06-21T12:00:00Z",
    north: 0,
    intensity: 3,
    shadows: true,
    computed: null,
  },
  ambient: 0.6,
};

/** Physically based material presets (generic surfaces, approximate values). */
export const PRESETS: Record<string, Omit<MaterialDef, "preset">> = {
  asphalt: { color: "#3b3b3d", roughness: 0.95, metalness: 0, opacity: 1 },
  concrete: { color: "#9c9a94", roughness: 0.9, metalness: 0, opacity: 1 },
  plaster: { color: "#e4e0d8", roughness: 0.85, metalness: 0, opacity: 1 },
  brick: { color: "#8e4a36", roughness: 0.9, metalness: 0, opacity: 1 },
  wood: { color: "#8a6440", roughness: 0.7, metalness: 0, opacity: 1 },
  painted_metal: { color: "#2f5a8a", roughness: 0.35, metalness: 0.6, opacity: 1 },
  metal: { color: "#b8bcc2", roughness: 0.3, metalness: 1, opacity: 1 },
  glass: { color: "#9fc4d8", roughness: 0.05, metalness: 0, opacity: 0.35 },
  rubber: { color: "#1c1c1c", roughness: 0.9, metalness: 0, opacity: 1 },
  fabric: { color: "#5a6270", roughness: 1, metalness: 0, opacity: 1 },
  skin: { color: "#c49a7c", roughness: 0.6, metalness: 0, opacity: 1 },
  road_paint: { color: "#f2f2ee", roughness: 0.6, metalness: 0, opacity: 1 },
  marker: { color: "#f2c94c", roughness: 0.5, metalness: 0, opacity: 1 },
  grass: { color: "#5d7a3a", roughness: 1, metalness: 0, opacity: 1 },
  roof_tile: { color: "#6b3a2e", roughness: 0.8, metalness: 0, opacity: 1 },
};

export const preset = (name: keyof typeof PRESETS): MaterialDef => ({
  preset: name,
  ...PRESETS[name],
});

/** Default material for each part of an extrusion or slot of a model. */
export const DEFAULT_MATERIAL: Record<Part | Slot, MaterialDef> = {
  wall: preset("plaster"),
  floor: preset("concrete"),
  road: preset("asphalt"),
  shoulder: preset("concrete"),
  marking: preset("road_paint"),
  body: preset("painted_metal"),
  glass: preset("glass"),
  tyre: preset("rubber"),
  skin: preset("skin"),
  cloth: preset("fabric"),
  wood: preset("wood"),
  metal: preset("metal"),
  marker: preset("marker"),
};

/** Column-major 4×4 for a rotation about z by `heading` degrees, then a translation. */
export function placeMatrix(position: [number, number, number], heading: number): number[] {
  const t = (heading * Math.PI) / 180;
  const [c, s] = [Math.cos(t), Math.sin(t)];
  return [c, s, 0, 0, -s, c, 0, 0, 0, 0, 1, 0, position[0], position[1], position[2], 1];
}
