// The animation stored with a 3D scene: the serde shape of locus-analysis::animation
// (crates/locus-analysis/src/animation.rs), which evaluates it. Project frame, metres, seconds
// from the stated time zero. See docs/methods/animation.md.
import type { CrashRecord } from "../api";

type P3 = [number, number, number];

/** Where a segment's motion, a path or a friction value comes from. */
export type Source =
  | { kind: "edr"; analysis_id: number; name: string }
  | { kind: "analysis"; analysis_id: number; tool: string; name: string }
  | { kind: "evidence"; evidence_id: number; name: string; note: string }
  | { kind: "assumption"; note: string };

export type SegmentMotion =
  | { kind: "speed"; speed: number }
  | { kind: "accelerate"; acceleration: number; start_speed: number | null }
  | { kind: "table"; times: number[]; distances: number[]; speeds?: number[] | null };

export interface Segment {
  /** Seconds; null (only the last) runs to the end of the timeline. */
  duration: number | null;
  motion: SegmentMotion;
  source: Source;
}

export interface Friction {
  mu: number;
  tolerance: number;
  source: Source;
}

export type MoverKind =
  { kind: "vehicle"; wheelbase: number } | { kind: "person" } | { kind: "other" };

export interface Mover {
  id: string;
  name: string;
  /** The scene object it moves. */
  object: string | null;
  kind: MoverKind;
  path: P3[];
  shape: "smooth" | "straight";
  path_source: Source;
  start: number;
  offset: number;
  segments: Segment[];
  friction: Friction | null;
}

export type ViewKind =
  | { kind: "driver"; mover: string; eye: P3 }
  | { kind: "witness"; floor: P3; eye_height: number; target: P3 };

export interface View {
  id: string;
  name: string;
  kind: ViewKind;
  hfov_deg: number;
  source: Source;
}

export interface Animation {
  time_zero: { event: string; basis: string };
  from: number;
  to: number;
  lighting: "daylight" | "low_light";
  movers: Mover[];
  views: View[];
}

export interface Sample {
  t: number;
  position: P3;
  /** Radians anticlockwise from +x. */
  heading: number;
  distance: number;
  speed: number;
  longitudinal: number;
  lateral: number;
  segment: number | null;
  assumed: boolean;
}

export interface Flag {
  mover: string;
  kind: "friction" | "friction_limit" | "speed_jump" | "heading_jump" | "no_friction";
  from: number;
  to: number;
  peak: number;
  limit: number;
  message: string;
}

export interface Evaluation {
  step: number;
  samples: [string, Sample[]][];
  flags: Flag[];
  assumed: { mover: string; segment: number | null; from: number; to: number; note: string }[];
  warnings: string[];
  limitations: string[];
}

/** Playback and plausibility sampling interval (s): 100 Hz, as the checks use. */
export const STEP = 0.01;

export const HUMAN_HFOV_DEG = 60;

export const NEW_ANIMATION: Animation = {
  time_zero: { event: "", basis: "" },
  from: -3,
  to: 2,
  lighting: "daylight",
  movers: [],
  views: [],
};

export const assumption = (note = ""): Source => ({ kind: "assumption", note });

export function describe(s: Source): string {
  switch (s.kind) {
    case "edr":
      return `EDR record "${s.name}"`;
    case "analysis":
      return `${s.tool} analysis "${s.name}"`;
    case "evidence":
      return `measured on ${s.name}${s.note ? `: ${s.note}` : ""}`;
    case "assumption":
      return `assumed${s.note ? `: ${s.note}` : ""}`;
  }
}

/** The sample at time `t`, interpolated between the two nearest (samples start at `from`). */
export function sampleAt(samples: Sample[], from: number, step: number, t: number): Sample {
  const x = Math.min(Math.max((t - from) / step, 0), samples.length - 1);
  const i = Math.min(Math.floor(x), samples.length - 2);
  if (i < 0) return samples[0];
  const [a, b] = [samples[i], samples[i + 1]];
  const u = x - i;
  const lerp = (p: number, q: number) => p + (q - p) * u;
  let dh = b.heading - a.heading;
  dh -= 2 * Math.PI * Math.round(dh / (2 * Math.PI));
  return {
    ...(u < 0.5 ? a : b),
    t,
    position: [0, 1, 2].map((k) => lerp(a.position[k], b.position[k])) as P3,
    heading: a.heading + dh * u,
    distance: lerp(a.distance, b.distance),
    speed: lerp(a.speed, b.speed),
  };
}

/**
 * A model's matrix (column-major, project frame) for a sample. A vehicle's sample is its rear
 * axle's centre; its model's origin is the body's centre, `rearAxle` (m, negative) from it
 * along +x. Other models stand on the sample.
 */
export function poseMatrix(s: Sample, rearAxle = 0): number[] {
  const [c, n] = [Math.cos(s.heading), Math.sin(s.heading)];
  const p = s.position;
  return [c, n, 0, 0, -n, c, 0, 0, 0, 0, 1, 0, p[0] - c * rearAxle, p[1] - n * rearAxle, p[2], 1];
}

/**
 * A segment from an EDR record: its stations' distance to the record's end, as distance from
 * its first sample against time from it. The mover starts at the first sample's time, and on
 * the record's own path (straight segments, as the EDR analysis measures it) that far back
 * from its end.
 */
export function edrSegment(
  r: CrashRecord,
): { segment: Segment; start: number; path: P3[]; offset: number } | null {
  const st = r.record.stations;
  if (!st || st.length < 2) return null;
  const [t0, d0] = [st[0].t, st[0].distance.value];
  const path = r.record.path ?? [];
  let length = 0;
  for (let i = 1; i < path.length; i++)
    length += Math.hypot(...[0, 1, 2].map((k) => path[i][k] - path[i - 1][k]));
  return {
    start: t0,
    path,
    offset: length - d0,
    segment: {
      duration: st[st.length - 1].t - t0,
      motion: {
        kind: "table",
        times: st.map((s) => s.t - t0),
        distances: st.map((s) => d0 - s.distance.value),
        // The record's speeds set the slopes: constant deceleration between samples.
        speeds:
          r.record.samples?.length === st.length ? r.record.samples.map((x) => x.speed) : null,
      },
      source: { kind: "edr", analysis_id: r.id, name: r.name },
    },
  };
}
