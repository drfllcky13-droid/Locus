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

/** Ranges are (low, high), when the segment's source gives them. */
export type SegmentMotion =
  | { kind: "speed"; speed: number; range?: [number, number] | null }
  | {
      kind: "accelerate";
      acceleration: number;
      start_speed: number | null;
      range?: [number, number] | null;
    }
  | {
      kind: "table";
      times: number[];
      distances: number[];
      speeds?: number[] | null;
      ranges?: [number, number][] | null;
      speed_ranges?: [number, number][] | null;
    };

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

/** Driver and witness views are people's (a human-like field of view); orbit and follow are
 * presentation cameras. Driver eye and follow offset are in the mover's frame: forward, left,
 * up (m). */
export type ViewKind =
  | { kind: "driver"; mover: string; eye: P3 }
  | { kind: "witness"; floor: P3; eye_height: number; target: P3; target_mover: string | null }
  | { kind: "orbit"; centre: P3; radius: number; height: number; period: number }
  | { kind: "follow"; mover: string; offset: P3; look_ahead: number }
  | {
      kind: "fly_through";
      points: P3[];
      shape: "smooth" | "straight";
      speed: number;
      start: number;
      look_ahead: number;
      target: P3 | null;
    }
  | { kind: "mirror"; mover: string; eye: P3; mirror: P3; normal: P3; width: number }
  | { kind: "panorama"; at: P3; mover: string | null; eye: P3 };

export const humanView = (k: ViewKind) => k.kind === "driver" || k.kind === "witness";

export interface Camera {
  eye: P3;
  target: P3;
  /** Horizontal field of view (°); 360 for a panorama. */
  hfov_deg: number;
}

const unit3 = (a: P3): P3 => {
  const l = Math.hypot(...a);
  return [a[0] / l, a[1] / l, a[2] / l];
};

/**
 * A flat mirror's normal (vehicle frame) that shows the eye the view `outwardDeg` out from
 * straight back, on the mirror's side: the bisector of the directions to the eye and to that
 * view.
 */
export function mirrorNormal(eye: P3, mirror: P3, outwardDeg: number): P3 {
  const side = Math.sign(mirror[1]) || 1;
  const a = (outwardDeg * Math.PI) / 180;
  const back: P3 = [-Math.cos(a), side * Math.sin(a), 0];
  const toEye = unit3([eye[0] - mirror[0], eye[1] - mirror[1], eye[2] - mirror[2]]);
  return unit3([toEye[0] + back[0], toEye[1] + back[1], toEye[2] + back[2]]);
}

/** How far out from straight back (°) a flat mirror shows the eye (the inverse of
 * `mirrorNormal`, level part). */
export function mirrorOutward(eye: P3, mirror: P3, normal: P3): number {
  const n = unit3(normal);
  const v = unit3([mirror[0] - eye[0], mirror[1] - eye[1], mirror[2] - eye[2]]);
  const k = 2 * (v[0] * n[0] + v[1] * n[1] + v[2] * n[2]);
  const d = [v[0] - k * n[0], v[1] - k * n[1]];
  const side = Math.sign(mirror[1]) || 1;
  return (Math.atan2(side * d[1], -d[0]) * 180) / Math.PI;
}

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
  /** Per view id: its camera at the samples' times. */
  cameras: [string, Camera[]][];
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

/** The time–distance–speed report's request (locus-analysis tds::TdsRequest). */
export interface TdsRequest {
  step: number;
  pairs: [string, string][];
  closing: boolean;
  points: { name: string; position: P3; source: Source }[];
}

/** A view's camera at time `t`, interpolated like `sampleAt`. */
export function cameraAt(cams: Camera[], from: number, step: number, t: number): Camera {
  const x = Math.min(Math.max((t - from) / step, 0), cams.length - 1);
  const i = Math.min(Math.floor(x), cams.length - 2);
  if (i < 0) return cams[0];
  const u = x - i;
  const lerp = (p: P3, q: P3) => [0, 1, 2].map((k) => p[k] + (q[k] - p[k]) * u) as P3;
  return {
    eye: lerp(cams[i].eye, cams[i + 1].eye),
    target: lerp(cams[i].target, cams[i + 1].target),
    hfov_deg: cams[i].hfov_deg,
  };
}

/**
 * A driver's eye in a vehicle's frame when not measured: a typical seat, forward of the rear
 * axle by 45 % of the wheelbase, 0.35 m left of centre, 1.2 m up. Recorded as an assumption.
 */
export const defaultDriverEye = (wheelbase: number): P3 => [0.45 * wheelbase, 0.35, 1.2];

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
  const samples = r.record.samples?.length === st.length ? r.record.samples : null;
  const [scale, offset] = [r.record.scale_tolerance ?? 0, r.record.offset_tolerance ?? 0];
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
        speeds: samples ? samples.map((x) => x.speed) : null,
        // Where the record puts it: its distance range (range method) from the first sample's
        // nominal distance, and its speed tolerance (systematic scale and offset).
        ranges: st.map((s) => [d0 - s.distance.high, d0 - s.distance.low] as [number, number]),
        speed_ranges: samples
          ? samples.map(
              (x) =>
                [Math.max(0, x.speed * (1 - scale) - offset), x.speed * (1 + scale) + offset] as [
                  number,
                  number,
                ],
            )
          : null,
      },
      source: { kind: "edr", analysis_id: r.id, name: r.name },
    },
  };
}
