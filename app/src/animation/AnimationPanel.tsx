// The animation editor and timeline, under the 3D scene builder: time zero and lighting,
// movers on paths picked on the cloud with motion segments, each with its source, and a
// timeline that plays the evaluated motion (locus-analysis animation) and marks assumed
// segments and plausibility flags. Stored in the scene document, so saved and audit-logged
// with it. See docs/methods/animation.md.
import { useEffect, useMemo, useState } from "react";
import * as THREE from "three";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  type AnalysisRecord,
  type CrashRecord,
  type EvidenceRecord,
  type Spread,
} from "../api";
import type { SceneDoc, SceneObject } from "../scene3d/model";
import type { Engine } from "../viewer3d/engine";
import type { PickHit } from "../viewer3d/pointcloud";
import {
  assumption,
  cameraAt,
  defaultDriverEye,
  describe,
  mirrorNormal,
  mirrorOutward,
  edrSegment,
  HUMAN_HFOV_DEG,
  humanView,
  NEW_ANIMATION,
  sampleAt,
  STEP,
  type Animation,
  type Evaluation,
  type Mover,
  type Sample,
  type Segment,
  type Source,
  type TdsRequest,
  type View,
} from "./model";
import { RenderSection } from "./RenderSection";
import { showAt } from "./show";
import { useReadOnly } from "../readOnly";

type Model = Extract<SceneObject, { kind: "model" }>;
type P3 = [number, number, number];

const num = (label: string, value: number, set: (v: number) => void, step = 0.1) => (
  <label>
    {label}
    <input
      type="number"
      value={Number(value.toFixed(6))}
      step={step}
      onChange={(e) => Number.isFinite(e.target.valueAsNumber) && set(e.target.valueAsNumber)}
    />
  </label>
);

const baseName = (e: EvidenceRecord) => e.original_path.split(/[\\/]/).pop() ?? `#${e.id}`;

function SourceEdit({
  label,
  value,
  onChange,
  analyses,
  evidence,
}: {
  label: string;
  value: Source;
  onChange: (s: Source) => void;
  analyses: AnalysisRecord[];
  evidence: EvidenceRecord[];
}) {
  const kind = value.kind === "edr" ? "analysis" : value.kind;
  return (
    <div className="anim-source">
      <label>
        {label}
        <select
          value={kind}
          onChange={(e) => {
            const k = e.target.value;
            if (k === "assumption") onChange(assumption());
            else if (k === "analysis" && analyses[0]) onChange(fromAnalysis(analyses[0]));
            else if (k === "evidence" && evidence[0])
              onChange({
                kind: "evidence",
                evidence_id: evidence[0].id,
                name: baseName(evidence[0]),
                note: "",
              });
          }}
        >
          <option value="assumption">Examiner&apos;s assumption</option>
          <option value="analysis" disabled={!analyses.length}>
            Analysis record
          </option>
          <option value="evidence" disabled={!evidence.length}>
            Measured on evidence
          </option>
        </select>
      </label>
      {(value.kind === "analysis" || value.kind === "edr") && (
        <select
          value={value.analysis_id}
          onChange={(e) => {
            const r = analyses.find((a) => a.id === Number(e.target.value));
            if (r) onChange(fromAnalysis(r));
          }}
        >
          {analyses.map((a) => (
            <option key={a.id} value={a.id}>
              {a.tool}: {a.name}
            </option>
          ))}
        </select>
      )}
      {value.kind === "evidence" && (
        <>
          <select
            value={value.evidence_id}
            onChange={(e) => {
              const r = evidence.find((x) => x.id === Number(e.target.value));
              if (r) onChange({ ...value, evidence_id: r.id, name: baseName(r) });
            }}
          >
            {evidence.map((e) => (
              <option key={e.id} value={e.id}>
                {baseName(e)}
              </option>
            ))}
          </select>
          <input
            placeholder="What was measured"
            value={value.note}
            onChange={(e) => onChange({ ...value, note: e.target.value })}
          />
        </>
      )}
      {value.kind === "assumption" && (
        <input
          placeholder="Why it is assumed (required)"
          value={value.note}
          className={value.note.trim() ? "" : "anim-missing"}
          onChange={(e) => onChange({ kind: "assumption", note: e.target.value })}
        />
      )}
    </div>
  );
}

const fromAnalysis = (r: AnalysisRecord): Source =>
  r.tool === "edr"
    ? { kind: "edr", analysis_id: r.id, name: r.name }
    : { kind: "analysis", analysis_id: r.id, tool: r.tool, name: r.name };

function SegmentEdit({
  seg,
  last,
  onChange,
  onRemove,
  analyses,
  evidence,
}: {
  seg: Segment;
  last: boolean;
  onChange: (s: Segment) => void;
  onRemove: () => void;
  analyses: AnalysisRecord[];
  evidence: EvidenceRecord[];
}) {
  const m = seg.motion;
  return (
    <div className={`anim-segment${seg.source.kind === "assumption" ? " assumed" : ""}`}>
      {m.kind === "speed" &&
        num(
          "Speed (km/h)",
          m.speed * 3.6,
          (v) => onChange({ ...seg, motion: { ...m, speed: v / 3.6 } }),
          1,
        )}
      {m.kind === "accelerate" && (
        <>
          {num("Acceleration (m/s², − brakes)", m.acceleration, (v) =>
            onChange({ ...seg, motion: { ...m, acceleration: v } }),
          )}
          <label className="inline">
            <input
              type="checkbox"
              checked={m.start_speed === null}
              onChange={(e) =>
                onChange({ ...seg, motion: { ...m, start_speed: e.target.checked ? null : 0 } })
              }
            />
            From the previous segment&apos;s speed
          </label>
          {m.start_speed !== null &&
            num(
              "Start speed (km/h)",
              m.start_speed * 3.6,
              (v) => onChange({ ...seg, motion: { ...m, start_speed: v / 3.6 } }),
              1,
            )}
        </>
      )}
      {m.kind === "table" && (
        <p className="muted">
          {m.times.length} time–distance points over {m.times[m.times.length - 1].toFixed(2)} s,{" "}
          {m.distances[m.distances.length - 1].toFixed(2)} m.
        </p>
      )}
      {m.kind !== "table" && (
        <>
          {last && (
            <label className="inline">
              <input
                type="checkbox"
                checked={seg.duration === null}
                onChange={(e) => onChange({ ...seg, duration: e.target.checked ? null : 1 })}
              />
              Runs to the end of the timeline
            </label>
          )}
          {seg.duration !== null &&
            num("Duration (s)", seg.duration, (v) => v > 0 && onChange({ ...seg, duration: v }))}
        </>
      )}
      <SourceEdit
        label="Source"
        value={seg.source}
        onChange={(source) => onChange({ ...seg, source })}
        analyses={analyses}
        evidence={evidence}
      />
      {(() => {
        // An analysis's speed (and its range-method range) for a constant-speed segment.
        const src = seg.source;
        const rec =
          src.kind === "analysis" ? analyses.find((x) => x.id === src.analysis_id) : undefined;
        const sp =
          rec && "speed" in rec.record ? (rec.record.speed as Spread | undefined) : undefined;
        if (m.kind !== "speed" || !sp) return null;
        return (
          <button
            onClick={() =>
              onChange({ ...seg, motion: { ...m, speed: sp.value, range: [sp.low, sp.high] } })
            }
          >
            Use its speed ({(sp.value * 3.6).toFixed(1)} km/h, {(sp.low * 3.6).toFixed(1)}–
            {(sp.high * 3.6).toFixed(1)})
          </button>
        );
      })()}
      {(m.kind === "speed" || m.kind === "accelerate") && m.range && (
        <p className="muted">
          Range {(m.range[0] * 3.6).toFixed(1)}–{(m.range[1] * 3.6).toFixed(1)} km/h{" "}
          <button onClick={() => onChange({ ...seg, motion: { ...m, range: null } })}>Clear</button>
        </p>
      )}
      <button onClick={onRemove}>Remove segment</button>
    </div>
  );
}

function MoverEdit({
  mover,
  onChange,
  onRemove,
  models,
  analyses,
  evidence,
  requestPick,
  onNotice,
  setPath,
}: {
  mover: Mover;
  onChange: (m: Mover) => void;
  /** Set the path alone (picks arrive after other edits may have landed). */
  setPath: (path: P3[]) => void;
  onRemove: () => void;
  models: Model[];
  analyses: AnalysisRecord[];
  evidence: EvidenceRecord[];
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  onNotice: (m: string | null) => void;
}) {
  const m = mover;
  const edrs = analyses.filter((a): a is CrashRecord => a.tool === "edr");
  const setSegs = (segments: Segment[]) => onChange({ ...m, segments });
  // Each pick appends a point; the next pick is asked for until the examiner stops.
  const pickMore = (path: P3[]) =>
    requestPick(`Click point ${path.length + 1} of ${m.name}'s path.`, (hit) => {
      api.pickResolve(hit).then(
        (r) => {
          const next = [...path, r.project];
          setPath(next);
          pickMore(next);
        },
        (e) => onNotice(String(e)),
      );
    });
  return (
    <div className="dg-built">
      <label>
        Name
        <input value={m.name} onChange={(e) => onChange({ ...m, name: e.target.value })} />
      </label>
      <label>
        Scene object
        <select
          value={m.object ?? ""}
          onChange={(e) => {
            const o = models.find((x) => x.id === e.target.value);
            const kind: Mover["kind"] =
              o?.asset.type === "vehicle"
                ? { kind: "vehicle", wheelbase: o.asset.wheelbase }
                : o?.asset.type === "person"
                  ? { kind: "person" }
                  : m.kind;
            onChange({ ...m, object: o?.id ?? null, kind });
          }}
        >
          <option value="">None (a marker)</option>
          {models.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name}
            </option>
          ))}
        </select>
      </label>
      <label>
        Kind
        <select
          value={m.kind.kind}
          onChange={(e) =>
            onChange({
              ...m,
              kind:
                e.target.value === "vehicle"
                  ? { kind: "vehicle", wheelbase: 2.7 }
                  : { kind: e.target.value as "person" | "other" },
            })
          }
        >
          <option value="vehicle">Vehicle (path is its rear axle)</option>
          <option value="person">Person</option>
          <option value="other">Other</option>
        </select>
      </label>
      {m.kind.kind === "vehicle" &&
        num(
          "Wheelbase (m)",
          m.kind.wheelbase,
          (wheelbase) => wheelbase > 0 && onChange({ ...m, kind: { kind: "vehicle", wheelbase } }),
          0.01,
        )}

      <h4>Path</h4>
      <p className="muted">
        {m.path.length} point{m.path.length === 1 ? "" : "s"}
        {m.kind.kind === "vehicle" ? ", at the rear axle's centre" : ""}.
      </p>
      <div className="row">
        <button onClick={() => pickMore(m.path)}>Pick points</button>
        <button
          disabled={!m.path.length}
          onClick={() => onChange({ ...m, path: m.path.slice(0, -1) })}
        >
          Undo point
        </button>
        <button disabled={!m.path.length} onClick={() => onChange({ ...m, path: [] })}>
          Clear
        </button>
      </div>
      <label>
        Shape
        <select
          value={m.shape}
          onChange={(e) => onChange({ ...m, shape: e.target.value as Mover["shape"] })}
        >
          <option value="smooth">Smooth curve through the points</option>
          <option value="straight">Straight segments</option>
        </select>
      </label>
      <SourceEdit
        label="Path source"
        value={m.path_source}
        onChange={(path_source) => onChange({ ...m, path_source })}
        analyses={analyses}
        evidence={evidence}
      />
      {num("Starts at (s)", m.start, (start) => onChange({ ...m, start }))}
      {num(
        "Starts along the path (m)",
        m.offset,
        (offset) => offset >= 0 && onChange({ ...m, offset }),
      )}

      <h4>Motion</h4>
      {m.segments.map((s, k) => (
        <SegmentEdit
          key={k}
          seg={s}
          last={k === m.segments.length - 1}
          onChange={(s2) => setSegs(m.segments.map((x, j) => (j === k ? s2 : x)))}
          onRemove={() => setSegs(m.segments.filter((_, j) => j !== k))}
          analyses={analyses}
          evidence={evidence}
        />
      ))}
      <label>
        Add a segment
        <select
          value=""
          onChange={(e) => {
            const v = e.target.value;
            // Only the last segment may run to the end; the one before it gets a duration.
            const segs = m.segments.map((s) => (s.duration === null ? { ...s, duration: 1 } : s));
            if (v === "speed")
              setSegs([
                ...segs,
                {
                  duration: 1,
                  motion: { kind: "speed", speed: m.kind.kind === "person" ? 1.4 : 10 },
                  source: assumption(),
                },
              ]);
            else if (v === "accelerate")
              setSegs([
                ...segs,
                {
                  duration: 1,
                  motion: { kind: "accelerate", acceleration: -7, start_speed: null },
                  source: assumption(),
                },
              ]);
            else if (v.startsWith("edr:")) {
              const r = edrs.find((x) => x.id === Number(v.slice(4)));
              const got = r && edrSegment(r);
              if (!got) return onNotice("That EDR record has no time–distance stations.");
              const first = !m.segments.length;
              const onPath = got.path.length >= 2 && (first || !m.path.length);
              if (onPath && got.offset < 0)
                onNotice(
                  `The EDR record covers ${(-got.offset).toFixed(1)} m more than its path; the vehicle starts at the path's start.`,
                );
              onChange({
                ...m,
                segments: [...segs, got.segment],
                ...(first ? { start: got.start } : {}),
                ...(onPath
                  ? {
                      path: got.path,
                      shape: "straight" as const,
                      offset: Math.max(0, got.offset),
                      path_source: got.segment.source,
                    }
                  : {}),
              });
            }
          }}
        >
          <option value="">Choose…</option>
          <option value="speed">Constant speed</option>
          <option value="accelerate">Constant acceleration or braking</option>
          {edrs.map((r) => (
            <option key={r.id} value={`edr:${r.id}`}>
              EDR record: {r.name}
            </option>
          ))}
        </select>
      </label>

      <h4>Friction</h4>
      <label className="inline">
        <input
          type="checkbox"
          checked={m.friction !== null}
          onChange={(e) =>
            onChange({
              ...m,
              friction: e.target.checked ? { mu: 0.7, tolerance: 0.1, source: assumption() } : null,
            })
          }
        />
        State the road&apos;s friction (checks the motion against it)
      </label>
      {m.friction && (
        <>
          {num(
            "μ",
            m.friction.mu,
            (mu) => mu > 0 && onChange({ ...m, friction: { ...m.friction!, mu } }),
            0.05,
          )}
          {num(
            "± tolerance",
            m.friction.tolerance,
            (tolerance) =>
              tolerance >= 0 && onChange({ ...m, friction: { ...m.friction!, tolerance } }),
            0.01,
          )}
          <SourceEdit
            label="Friction source"
            value={m.friction.source}
            onChange={(source) => onChange({ ...m, friction: { ...m.friction!, source } })}
            analyses={analyses}
            evidence={evidence}
          />
        </>
      )}
      <button onClick={onRemove}>Remove {m.name}</button>
    </div>
  );
}

function ViewEdit({
  view,
  movers,
  onChange,
  onRemove,
  analyses,
  evidence,
  pickPoint,
}: {
  view: View;
  movers: Mover[];
  onChange: (v: View) => void;
  onRemove: () => void;
  analyses: AnalysisRecord[];
  evidence: EvidenceRecord[];
  pickPoint: (hint: string, then: (p: P3) => void) => void;
}) {
  const v = view;
  const k = v.kind;
  const set = (kind: View["kind"]) => onChange({ ...v, kind });
  const xyz = (label: string, p: P3, to: (p: P3) => void, names: [string, string, string]) =>
    names.map((n, i) =>
      num(`${label} ${n} (m)`, p[i], (x) => to(p.map((y, j) => (j === i ? x : y)) as P3), 0.05),
    );
  const moverSelect = (label: string, value: string, to: (id: string) => void, only?: string) => (
    <label>
      {label}
      <select value={value} onChange={(e) => to(e.target.value)}>
        {movers
          .filter((m) => !only || m.kind.kind === only)
          .map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
      </select>
    </label>
  );
  const human = humanView(k);
  return (
    <div className="dg-built">
      <label>
        Name
        <input value={v.name} onChange={(e) => onChange({ ...v, name: e.target.value })} />
      </label>
      {k.kind === "driver" && (
        <>
          {moverSelect("Vehicle", k.mover, (mover) => set({ ...k, mover }), "vehicle")}
          {xyz("Eye", k.eye, (eye) => set({ ...k, eye }), [
            "forward of the rear axle",
            "left of centre",
            "up from the ground",
          ])}
        </>
      )}
      {k.kind === "witness" && (
        <>
          <p className="muted">Standing at {k.floor.map((x) => x.toFixed(2)).join(", ")}.</p>
          <button
            onClick={() =>
              pickPoint("Click where the witness stood.", (floor) => set({ ...k, floor }))
            }
          >
            Pick where they stood
          </button>
          {num(
            "Eye height (m)",
            k.eye_height,
            (eye_height) => eye_height > 0 && set({ ...k, eye_height }),
            0.01,
          )}
          <label>
            Looking at
            <select
              value={k.target_mover ?? ""}
              onChange={(e) => set({ ...k, target_mover: e.target.value || null })}
            >
              <option value="">A point</option>
              {movers.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name} (tracked)
                </option>
              ))}
            </select>
          </label>
          {k.target_mover === null && (
            <button
              onClick={() =>
                pickPoint("Click what the witness looked at.", (target) => set({ ...k, target }))
              }
            >
              Pick the point
            </button>
          )}
        </>
      )}
      {k.kind === "orbit" && (
        <>
          <button
            onClick={() =>
              pickPoint("Click the orbit's centre.", (centre) => set({ ...k, centre }))
            }
          >
            Pick the centre
          </button>
          {num("Radius (m)", k.radius, (radius) => radius > 0 && set({ ...k, radius }), 0.5)}
          {num("Height (m)", k.height, (height) => set({ ...k, height }), 0.5)}
          {num(
            "Once around every (s)",
            k.period,
            (period) => period > 0 && set({ ...k, period }),
            1,
          )}
        </>
      )}
      {k.kind === "follow" && (
        <>
          {moverSelect("Mover", k.mover, (mover) => set({ ...k, mover }))}
          {xyz("Camera", k.offset, (offset) => set({ ...k, offset }), [
            "forward (− behind)",
            "left",
            "up",
          ])}
          {num("Looking ahead (m)", k.look_ahead, (look_ahead) => set({ ...k, look_ahead }), 0.5)}
        </>
      )}
      {k.kind === "fly_through" && (
        <>
          <p className="muted">{k.points.length} points on the camera's path.</p>
          <div className="row">
            <button
              onClick={() => {
                // Each pick adds a point and asks for the next, until Esc.
                const more = (pts: P3[]) =>
                  pickPoint(`Click point ${pts.length + 1} of the camera's path.`, (p) => {
                    const next = [...pts, [p[0], p[1], p[2] + 1.6] as P3];
                    set({ ...k, points: next });
                    more(next);
                  });
                more(k.points);
              }}
            >
              Pick points (1.6 m above)
            </button>
            <button disabled={!k.points.length} onClick={() => set({ ...k, points: [] })}>
              Clear
            </button>
          </div>
          {num("Speed (m/s)", k.speed, (speed) => speed > 0 && set({ ...k, speed }), 0.5)}
          {num("Starts at (s)", k.start, (start) => set({ ...k, start }))}
          {num(
            "Looking ahead along it (m)",
            k.look_ahead,
            (look_ahead) => set({ ...k, look_ahead }),
            1,
          )}
          <button
            onClick={() =>
              k.target
                ? set({ ...k, target: null })
                : pickPoint("Click what the camera looks at.", (target) => set({ ...k, target }))
            }
          >
            {k.target ? "Look along the path instead" : "Look at a fixed point…"}
          </button>
        </>
      )}
      {k.kind === "mirror" && (
        <>
          {moverSelect("Vehicle", k.mover, (mover) => set({ ...k, mover }), "vehicle")}
          {xyz("Eye", k.eye, (eye) => set({ ...k, eye }), [
            "forward of the rear axle",
            "left",
            "up",
          ])}
          {xyz("Mirror centre", k.mirror, (mirror) => set({ ...k, mirror }), [
            "forward of the rear axle",
            "left (− right)",
            "up",
          ])}
          {num("Mirror width (m)", k.width, (width) => width > 0 && set({ ...k, width }), 0.01)}
          {num(
            "Shows the view out from straight back (°)",
            mirrorOutward(k.eye, k.mirror, k.normal),
            (deg) => set({ ...k, normal: mirrorNormal(k.eye, k.mirror, deg) }),
            1,
          )}
          <p className="muted">
            A flat mirror; convex mirrors aren&apos;t modelled. The field of view is the
            mirror&apos;s width from the eye, and the image is reversed, as in a mirror.
          </p>
        </>
      )}
      {k.kind === "panorama" && (
        <>
          <label>
            From
            <select
              value={k.mover ?? ""}
              onChange={(e) => set({ ...k, mover: e.target.value || null })}
            >
              <option value="">A fixed point</option>
              {movers.map((m) => (
                <option key={m.id} value={m.id}>
                  A seat in {m.name}
                </option>
              ))}
            </select>
          </label>
          {k.mover === null ? (
            <>
              <p className="muted">At {k.at.map((x) => x.toFixed(2)).join(", ")}.</p>
              <button
                onClick={() =>
                  pickPoint("Click the floor under the 360° camera.", (p) =>
                    set({ ...k, at: [p[0], p[1], p[2] + 1.6] }),
                  )
                }
              >
                Pick the point (1.6 m above)
              </button>
            </>
          ) : (
            xyz("Eye", k.eye, (eye) => set({ ...k, eye }), [
              "forward of the rear axle",
              "left",
              "up",
            ])
          )}
          <p className="muted">
            A 360° image for a 360° viewer, centred on{" "}
            {k.mover === null ? "the project's +y" : "the mover's heading"}; not a person&apos;s
            field of view.
          </p>
        </>
      )}
      {k.kind !== "mirror" &&
        k.kind !== "panorama" &&
        num(
          "Horizontal field of view (°)",
          v.hfov_deg,
          (hfov_deg) => hfov_deg > 1 && hfov_deg < 179 && onChange({ ...v, hfov_deg }),
          1,
        )}
      {human && v.hfov_deg > HUMAN_HFOV_DEG && (
        <p className="error">
          Wider than the {HUMAN_HFOV_DEG}° human-like default: things look farther away and smaller
          than a person there would see them.
        </p>
      )}
      {human ? (
        <SourceEdit
          label="Eye position source"
          value={v.source}
          onChange={(source) => onChange({ ...v, source })}
          analyses={analyses}
          evidence={evidence}
        />
      ) : (
        <p className="muted">A presentation camera: nobody&apos;s point of view.</p>
      )}
      <button onClick={onRemove}>Remove {v.name}</button>
    </div>
  );
}

/** Runs of consecutive samples in one segment: [from, to, segment, assumed]. */
function runs(samples: Sample[]): [number, number, number | null, boolean][] {
  const out: [number, number, number | null, boolean][] = [];
  for (const s of samples) {
    const last = out[out.length - 1];
    if (last && last[2] === s.segment && last[3] === s.assumed) last[1] = s.t;
    else out.push([s.t, s.t, s.segment, s.assumed]);
  }
  return out;
}

function Timeline({
  a,
  ev,
  t,
  setT,
}: {
  a: Animation;
  ev: Evaluation;
  t: number;
  setT: (t: number) => void;
}) {
  const span = a.to - a.from;
  const pct = (x: number) => `${(((x - a.from) / span) * 100).toFixed(3)}%`;
  const at = (e: React.PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setT(a.from + Math.min(Math.max((e.clientX - r.left) / r.width, 0), 1) * span);
  };
  const ticks = [];
  const every = span > 20 ? 5 : span > 8 ? 2 : 1;
  for (let x = Math.ceil(a.from / every) * every; x <= a.to + 1e-9; x += every) ticks.push(x);
  return (
    <div
      className="anim-timeline"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        at(e);
      }}
      onPointerMove={(e) => e.buttons && at(e)}
    >
      <div className="anim-ruler">
        {ticks.map((x) => (
          <span key={x} style={{ left: pct(x) }} className={x === 0 ? "zero" : ""}>
            {x}
          </span>
        ))}
      </div>
      {ev.samples.map(([id, samples]) => {
        const m = a.movers.find((x) => x.id === id);
        return (
          <div key={id} className="anim-row" title={m?.name}>
            <span className="anim-row-name">{m?.name}</span>
            {runs(samples)
              .filter((r) => r[2] !== null)
              .map(([f, to, k, assumed]) => (
                <div
                  key={`${f}`}
                  className={`anim-seg${assumed ? " assumed" : ""}`}
                  style={{ left: pct(f), width: `${((to - f + ev.step) / span) * 100}%` }}
                  title={`Segment ${k! + 1}: ${m ? describe(m.segments[k!].source) : ""}${assumed ? " (illustrative)" : ""}`}
                />
              ))}
            {ev.flags
              .filter((f) => f.mover === m?.name)
              .map((f, i) => (
                <div
                  key={i}
                  className={`anim-flag ${f.kind}`}
                  style={{
                    left: pct(f.from),
                    width: `max(3px, ${((f.to - f.from) / span) * 100}%)`,
                  }}
                  title={f.message}
                />
              ))}
          </div>
        );
      })}
      <div className="anim-playhead" style={{ left: pct(t) }} />
    </div>
  );
}

export function AnimationPanel({
  engine,
  sceneId,
  saving,
  doc,
  setAnimation,
  evidence,
  requestPick,
  onNotice,
}: {
  engine: () => Engine | null;
  sceneId: number;
  /** Edits are still waiting to be saved (the report reads the saved revision). */
  saving: boolean;
  doc: SceneDoc;
  /** Update the scene's animation from its newest state. */
  setAnimation: (f: (a: Animation | undefined) => Animation | undefined) => void;
  evidence: EvidenceRecord[];
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  onNotice: (m: string | null) => void;
}) {
  const a = doc.animation;
  const set = (next: Animation | undefined) => setAnimation(() => next);
  const [analyses, setAnalyses] = useState<AnalysisRecord[]>([]);
  const [ev, setEv] = useState<Evaluation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [t, setT] = useState(a?.from ?? 0);
  const [playing, setPlaying] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const models = doc.objects.filter((o): o is Model => o.kind === "model");

  useEffect(() => {
    api.analyses().then(
      (r) => setAnalyses(r.filter((x) => !x.withdrawn)),
      (e) => onNotice(String(e)),
    );
  }, [onNotice]);

  // Movers still being built (no path or motion yet) are left out of the evaluation.
  // Views tied to such a mover wait for it too.
  const ready = useMemo(() => {
    if (!a) return null;
    const movers = a.movers.filter((m) => m.path.length >= 2 && m.segments.length);
    const ok = (id: string | null) => id === null || movers.some((m) => m.id === id);
    // The mover a view is tied to, if any; a fly-through needs its path first.
    const tied = (k: View["kind"]) =>
      k.kind === "driver" || k.kind === "follow" || k.kind === "mirror"
        ? k.mover
        : k.kind === "witness"
          ? k.target_mover
          : k.kind === "panorama"
            ? k.mover
            : null;
    const views = a.views.filter(
      (v) => ok(tied(v.kind)) && (v.kind.kind !== "fly_through" || v.kind.points.length >= 2),
    );
    return { ...a, movers, views };
  }, [a]);
  const readOnly = useReadOnly();
  const [through, setThrough] = useState<string>("");
  // Bumped after a render, which leaves the view at its last frame.
  const [redraw, setRedraw] = useState(0);
  const [tds, setTds] = useState<TdsRequest>({ step: 0.5, pairs: [], closing: true, points: [] });
  const [pair, setPair] = useState<[string, string]>(["", ""]);
  const [reports, setReports] = useState<AnalysisRecord[]>([]);
  const [editingView, setEditingView] = useState<string | null>(null);
  useEffect(() => {
    if (!ready) return setEv(null);
    const h = setTimeout(
      () =>
        api.animationEvaluate(ready, STEP).then(
          (e) => {
            setEv(e);
            setError(null);
          },
          (e) => setError(String(e)),
        ),
      150,
    );
    return () => clearTimeout(h);
  }, [ready]);

  // Keep the playhead in the range.
  useEffect(() => {
    if (a && (t < a.from || t > a.to)) setT(Math.min(Math.max(t, a.from), a.to));
  }, [a, t]);

  // Play: real time, looping.
  useEffect(() => {
    if (!playing || !a) return;
    let last = performance.now();
    let id = requestAnimationFrame(function tick(now) {
      const dt = (now - last) / 1000;
      last = now;
      setT((x) => (x + dt > a.to ? a.from : x + dt));
      id = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(id);
  }, [playing, a]);

  const now = useMemo(
    () =>
      ev && a
        ? ev.samples.map(([id, s]) => [id, sampleAt(s, a.from, ev.step, t)] as [string, Sample])
        : [],
    [ev, a, t],
  );

  // Looking through a view: the camera follows it; leaving restores the normal lens.
  const cams = ev?.cameras.find(([id]) => id === through)?.[1];
  const throughKind = a?.views.find((v) => v.id === through)?.kind.kind;
  useEffect(() => {
    const e = engine();
    if (!e || !a || !ev || !cams) return;
    const c = cameraAt(cams, a.from, ev.step, t);
    // A 360° view is looked through straight ahead at 90°; the render gives the whole sphere.
    e.setView(c.eye, c.target, cams[0].hfov_deg >= 360 ? 90 : cams[0].hfov_deg);
  }, [engine, a, ev, cams, t]);
  useEffect(() => {
    engine()?.setMirrored(throughKind === "mirror");
    if (through) return;
    const e = engine();
    const c = e?.cameraProject();
    if (e && c) e.setView(c.eye, c.target, null);
  }, [engine, through, throughKind]);
  useEffect(() => () => engine()?.setMirrored(false), [engine]);

  // Linked scene objects follow their mover; the rest show as markers.
  useEffect(() => {
    const e = engine();
    if (e && a) showAt(e, a, models, now, through);
  }, [now, a, models, engine, through, redraw]);

  // Each mover's path as travelled: measured blue, assumed orange, flagged stretches red.
  useEffect(() => {
    const e = engine();
    if (!e) return;
    if (!ev || !a) return e.setAnalysisOverlay("animation", null);
    const o = e.origin;
    const g = new THREE.Group();
    for (const [id, samples] of ev.samples) {
      const name = a.movers.find((x) => x.id === id)?.name;
      const flags = ev.flags.filter((f) => f.mover === name && f.kind !== "no_friction");
      const pos: number[] = [];
      const col: number[] = [];
      for (let i = 1; i < samples.length; i++) {
        const s = samples[i];
        const c = flags.some((f) => s.t >= f.from && s.t <= f.to)
          ? [0.95, 0.2, 0.2]
          : s.assumed
            ? [0.95, 0.6, 0.2]
            : [0.2, 0.75, 0.95];
        for (const p of [samples[i - 1].position, s.position]) {
          pos.push(p[0] - o[0], p[1] - o[1], p[2] - o[2] + 0.05);
          col.push(...c);
        }
      }
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
      geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
      g.add(new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ vertexColors: true })));
    }
    e.setAnalysisOverlay("animation", g);
  }, [ev, a, engine]);

  useEffect(
    () => () => {
      const e = engine();
      e?.setPoses(new Map());
      e?.setAnalysisOverlay("animation", null);
      e?.setAnalysisOverlay("animation-now", null);
    },
    [engine],
  );

  if (!a && readOnly) return null;
  if (!a)
    return (
      <section className="panel-section anim">
        <h3>Animation</h3>
        <p className="muted">
          Move vehicles and people along paths picked on the cloud, from EDR records, other
          analyses, measurements or stated assumptions.
        </p>
        <button onClick={() => set(NEW_ANIMATION)}>Start an animation</button>
      </section>
    );

  const upd = (m: Mover) => set({ ...a, movers: a.movers.map((x) => (x.id === m.id ? m : x)) });
  const incomplete = a.movers.filter((m) => !ready?.movers.includes(m));
  const sel = a.movers.find((m) => m.id === editing) ?? null;
  const attention = [...(ev?.flags ?? []).map((f) => f.message), ...(ev?.warnings ?? [])];

  return (
    <section className="panel-section anim">
      <h3>Animation</h3>
      {readOnly ? (
        <p className="muted">
          Time zero: {a.time_zero.event || "not stated"} (known from{" "}
          {a.time_zero.basis || "not stated"}). Timeline {a.from.toFixed(2)} to {a.to.toFixed(2)} s;{" "}
          {a.lighting === "low_light" ? "low light" : "daylight"}.
        </p>
      ) : (
        <>
          <label>
            Time zero is
            <input
              placeholder="The event, e.g. EDR trigger, impact"
              value={a.time_zero.event}
              className={a.time_zero.event.trim() ? "" : "anim-missing"}
              onChange={(e) => set({ ...a, time_zero: { ...a.time_zero, event: e.target.value } })}
            />
          </label>
          <label>
            Known from
            <input
              placeholder="How it is known, e.g. the EDR record"
              value={a.time_zero.basis}
              className={a.time_zero.basis.trim() ? "" : "anim-missing"}
              onChange={(e) => set({ ...a, time_zero: { ...a.time_zero, basis: e.target.value } })}
            />
          </label>
          <div className="row">
            {num("From (s)", a.from, (from) => from < a.to && set({ ...a, from }))}
            {num("To (s)", a.to, (to) => to > a.from && set({ ...a, to }))}
          </div>
          <label>
            Lighting
            <select
              value={a.lighting}
              onChange={(e) => set({ ...a, lighting: e.target.value as Animation["lighting"] })}
            >
              <option value="daylight">Daylight</option>
              <option value="low_light">Low light (dusk, dawn, night, artificial only)</option>
            </select>
          </label>
        </>
      )}

      <div className="row">
        <button className="primary" disabled={!ev} onClick={() => setPlaying((p) => !p)}>
          {playing ? "Pause" : "Play"}
        </button>
        <button
          onClick={() => {
            setPlaying(false);
            setT(a.from);
          }}
        >
          To start
        </button>
        <span className="anim-clock">
          {t >= 0 ? "+" : "−"}
          {Math.abs(t).toFixed(2)} s
        </span>
      </div>
      {ev && <Timeline a={a} ev={ev} t={t} setT={(x) => (setPlaying(false), setT(x))} />}
      {now.map(([id, s]) => {
        const m = a.movers.find((x) => x.id === id);
        return (
          <p key={id} className="muted anim-readout">
            {m?.name}: {(s.speed * 3.6).toFixed(1)} km/h ({s.speed.toFixed(2)} m/s),{" "}
            {s.distance.toFixed(2)} m along its path
            {s.assumed ? " · assumed (illustrative)" : ""}
          </p>
        );
      })}
      {error && <p className="error">{error}</p>}
      {attention.length > 0 && (
        <div className="anim-attention">
          <strong>Needs attention</strong>
          <ul>
            {attention.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}
      {ev && ev.assumed.length > 0 && (
        <details>
          <summary>Assumed ({ev.assumed.length})</summary>
          <ul>
            {ev.assumed.map((x, i) => (
              <li key={i}>
                {x.mover}
                {x.segment === null ? "" : `, segment ${x.segment + 1}`} ({x.from.toFixed(2)} to{" "}
                {x.to.toFixed(2)} s): {x.note || "no reason stated"}
              </li>
            ))}
          </ul>
        </details>
      )}
      {incomplete.length > 0 && (
        <p className="muted">
          Not yet playing (needs a path of two or more points and a segment):{" "}
          {incomplete.map((m) => m.name).join(", ")}.
        </p>
      )}

      {!readOnly && (
        <>
          <h4>Movers</h4>
          {a.movers.map((m) => (
            <button
              key={m.id}
              className={m.id === editing ? "primary" : ""}
              onClick={() => setEditing(m.id === editing ? null : m.id)}
            >
              {m.name}
            </button>
          ))}
          <button
            onClick={() => {
              const id = crypto.randomUUID();
              const n = a.movers.length + 1;
              set({
                ...a,
                movers: [
                  ...a.movers,
                  {
                    id,
                    name: `Mover ${n}`,
                    object: null,
                    kind: { kind: "vehicle", wheelbase: 2.7 },
                    path: [],
                    shape: "smooth",
                    path_source: assumption(),
                    start: a.from,
                    offset: 0,
                    segments: [],
                    friction: null,
                  },
                ],
              });
              setEditing(id);
            }}
          >
            Add a mover
          </button>
          {sel && (
            <MoverEdit
              mover={sel}
              onChange={upd}
              onRemove={() => {
                set({ ...a, movers: a.movers.filter((x) => x.id !== sel.id) });
                setEditing(null);
              }}
              models={models}
              analyses={analyses}
              evidence={evidence}
              requestPick={requestPick}
              onNotice={onNotice}
              setPath={(path) =>
                setAnimation(
                  (x) =>
                    x && {
                      ...x,
                      movers: x.movers.map((y) => (y.id === sel.id ? { ...y, path } : y)),
                    },
                )
              }
            />
          )}
        </>
      )}
      <h4>Views</h4>
      <label>
        Look through
        <select value={through} onChange={(e) => setThrough(e.target.value)}>
          <option value="">The normal camera</option>
          {a.views
            .filter((v) => ev?.cameras.some(([id]) => id === v.id))
            .map((v) => (
              <option key={v.id} value={v.id}>
                {v.name} ({v.hfov_deg.toFixed(0)}° horizontal)
              </option>
            ))}
        </select>
      </label>
      {!readOnly && (
        <>
          {a.views.map((v) => (
            <button
              key={v.id}
              className={v.id === editingView ? "primary" : ""}
              onClick={() => setEditingView(v.id === editingView ? null : v.id)}
            >
              {v.name}
            </button>
          ))}
          <label>
            Add a view
            <select
              value=""
              onChange={(e) => {
                const kind = e.target.value;
                const id = crypto.randomUUID();
                const cam = engine()?.cameraProject();
                const here = (cam?.target ?? [0, 0, 0]) as P3;
                const vehicle = a.movers.find((m) => m.kind.kind === "vehicle");
                const first = a.movers[0];
                const n = a.views.length + 1;
                let v: View | null = null;
                if (kind === "driver" && vehicle && vehicle.kind.kind === "vehicle") {
                  const eye = defaultDriverEye(vehicle.kind.wheelbase);
                  v = {
                    id,
                    name: `Driver of ${vehicle.name}`,
                    kind: { kind: "driver", mover: vehicle.id, eye },
                    hfov_deg: HUMAN_HFOV_DEG,
                    source: assumption(
                      `default eye position (${eye.map((x) => x.toFixed(2)).join(", ")} m forward, left, up from the rear axle): a typical driver's seat, not measured`,
                    ),
                  };
                } else if (kind === "witness")
                  v = {
                    id,
                    name: `Witness ${n}`,
                    kind: {
                      kind: "witness",
                      floor: here,
                      eye_height: 1.6,
                      target: here,
                      target_mover: first?.id ?? null,
                    },
                    hfov_deg: HUMAN_HFOV_DEG,
                    source: assumption("default eye height 1.6 m, not measured"),
                  };
                else if (kind === "orbit")
                  v = {
                    id,
                    name: `Orbit ${n}`,
                    kind: { kind: "orbit", centre: here, radius: 20, height: 10, period: 20 },
                    hfov_deg: HUMAN_HFOV_DEG,
                    source: assumption("presentation camera"),
                  };
                else if (kind === "follow" && first)
                  v = {
                    id,
                    name: `Following ${first.name}`,
                    kind: { kind: "follow", mover: first.id, offset: [-8, 0, 3], look_ahead: 5 },
                    hfov_deg: HUMAN_HFOV_DEG,
                    source: assumption("presentation camera"),
                  };
                else if (kind === "fly_through")
                  v = {
                    id,
                    name: `Fly-through ${n}`,
                    kind: {
                      kind: "fly_through",
                      points: [],
                      shape: "smooth",
                      speed: 3,
                      start: a.from,
                      look_ahead: 5,
                      target: null,
                    },
                    hfov_deg: HUMAN_HFOV_DEG,
                    source: assumption("presentation camera"),
                  };
                else if (kind.startsWith("mirror") && vehicle && vehicle.kind.kind === "vehicle") {
                  const eye = defaultDriverEye(vehicle.kind.wheelbase);
                  const side = kind === "mirror_left" ? 1 : -1;
                  const mirror: P3 = [eye[0] + 0.7, side * 1.0, 1.05];
                  v = {
                    id,
                    name: `${side > 0 ? "Left" : "Right"} mirror of ${vehicle.name}`,
                    kind: {
                      kind: "mirror",
                      mover: vehicle.id,
                      eye,
                      mirror,
                      normal: mirrorNormal(eye, mirror, 10),
                      width: 0.18,
                    },
                    hfov_deg: HUMAN_HFOV_DEG,
                    source: assumption(
                      "default eye and mirror positions (a typical seat; mirror 0.7 m ahead of the eye, 1.0 m out, 0.18 m wide), not measured",
                    ),
                  };
                } else if (kind === "panorama")
                  v = {
                    id,
                    name: `360° ${n}`,
                    kind: {
                      kind: "panorama",
                      at: [here[0], here[1], here[2] + 1.6],
                      mover: null,
                      eye: [1.2, 0.35, 1.2],
                    },
                    hfov_deg: HUMAN_HFOV_DEG,
                    source: assumption("presentation camera"),
                  };
                if (!v)
                  return onNotice("Add a mover (a vehicle, for a driver or mirror view) first.");
                set({ ...a, views: [...a.views, v] });
                setEditingView(id);
              }}
            >
              <option value="">Choose…</option>
              <option value="driver">Driver (from a vehicle)</option>
              <option value="witness">Witness (standing at a point)</option>
              <option value="orbit">Orbit (presentation)</option>
              <option value="follow">Follow a mover (presentation)</option>
              <option value="fly_through">Fly-through (presentation)</option>
              <option value="mirror_left">Left mirror (from a vehicle)</option>
              <option value="mirror_right">Right mirror (from a vehicle)</option>
              <option value="panorama">360° (for a 360° viewer)</option>
            </select>
          </label>
          {(() => {
            const v = a.views.find((x) => x.id === editingView);
            return (
              v && (
                <ViewEdit
                  view={v}
                  movers={a.movers}
                  onChange={(nv) =>
                    set({ ...a, views: a.views.map((x) => (x.id === nv.id ? nv : x)) })
                  }
                  onRemove={() => {
                    set({ ...a, views: a.views.filter((x) => x.id !== v.id) });
                    setEditingView(null);
                    if (through === v.id) setThrough("");
                  }}
                  analyses={analyses}
                  evidence={evidence}
                  pickPoint={(hint, then) =>
                    requestPick(hint, (hit) =>
                      api.pickResolve(hit).then(
                        (r) => then(r.project),
                        (e) => onNotice(String(e)),
                      ),
                    )
                  }
                />
              )
            );
          })()}
          <h4>Time, distance and speed report</h4>
          {num("Every (s)", tds.step, (step) => step >= 0.01 && setTds({ ...tds, step }), 0.1)}
          <div className="row">
            {[0, 1].map((i) => (
              <select
                key={i}
                value={pair[i]}
                onChange={(e) =>
                  setPair(i === 0 ? [e.target.value, pair[1]] : [pair[0], e.target.value])
                }
              >
                <option value="">Mover…</option>
                {a.movers.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            ))}
            <button
              disabled={!pair[0] || !pair[1] || pair[0] === pair[1]}
              onClick={() => {
                setTds({ ...tds, pairs: [...tds.pairs, pair] });
                setPair(["", ""]);
              }}
            >
              Add pair
            </button>
          </div>
          {tds.pairs.map(([x, y], i) => (
            <p key={i} className="muted">
              Distance {a.movers.find((m) => m.id === x)?.name} to{" "}
              {a.movers.find((m) => m.id === y)?.name}{" "}
              <button
                onClick={() => setTds({ ...tds, pairs: tds.pairs.filter((_, j) => j !== i) })}
              >
                Remove
              </button>
            </p>
          ))}
          <label className="inline">
            <input
              type="checkbox"
              checked={tds.closing}
              onChange={(e) => setTds({ ...tds, closing: e.target.checked })}
            />
            With closing speed
          </label>
          <button
            onClick={() =>
              requestPick("Click the point (a conflict point, a stop line).", (hit) =>
                api.pickResolve(hit).then(
                  (r) =>
                    setTds((x) => ({
                      ...x,
                      points: [
                        ...x.points,
                        {
                          name: `Point ${x.points.length + 1}`,
                          position: r.project,
                          source: assumption(),
                        },
                      ],
                    })),
                  (e) => onNotice(String(e)),
                ),
              )
            }
          >
            Add a point (time and distance to it)
          </button>
          {tds.points.map((p, i) => (
            <div key={i} className="dg-built">
              <label>
                Point name
                <input
                  value={p.name}
                  onChange={(e) =>
                    setTds({
                      ...tds,
                      points: tds.points.map((q, j) =>
                        j === i ? { ...q, name: e.target.value } : q,
                      ),
                    })
                  }
                />
              </label>
              <SourceEdit
                label="Why this point"
                value={p.source}
                onChange={(source) =>
                  setTds({
                    ...tds,
                    points: tds.points.map((q, j) => (j === i ? { ...q, source } : q)),
                  })
                }
                analyses={analyses}
                evidence={evidence}
              />
              <button
                onClick={() => setTds({ ...tds, points: tds.points.filter((_, j) => j !== i) })}
              >
                Remove point
              </button>
            </div>
          ))}
          <button
            className="primary"
            disabled={saving || !ev || !!error}
            title={saving ? "Waiting for the scene to save" : undefined}
            onClick={async () => {
              const name = window.prompt(
                "Name the report:",
                `Time, distance and speed ${reports.length + 1}`,
              );
              if (!name) return;
              try {
                const r = await api.animationSave(sceneId, name, tds);
                setReports((x) => [...x, r]);
                const path = await saveDialog({
                  defaultPath: `${name}.pdf`,
                  filters: [{ name: "PDF", extensions: ["pdf"] }],
                });
                if (!path)
                  return onNotice(`Saved as analysis ${r.id} (recorded in the audit log).`);
                const sha = await api.analysisReport(r.id, path);
                onNotice(
                  `Saved as analysis ${r.id}; report ${path} (SHA-256 ${sha}; recorded in the audit log).`,
                );
              } catch (e) {
                onNotice(String(e));
              }
            }}
          >
            Save and print the report…
          </button>
          {ready && (
            <RenderSection
              engine={engine}
              a={ready}
              models={models}
              sceneId={sceneId}
              saving={saving}
              onNotice={onNotice}
              onDone={() => setRedraw((x) => x + 1)}
            />
          )}
        </>
      )}
      {ev?.limitations.map((l, i) => (
        <p key={i} className="muted">
          {l}
        </p>
      ))}
    </section>
  );
}
