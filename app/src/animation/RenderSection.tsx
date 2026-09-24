// Rendering one of the animation's views to MP4: every frame is drawn from the evaluated motion
// at exactly its time, the overlays chosen (all off by default) and the view's permanent labels
// are drawn on it, and it goes to the Media Foundation writer. The file is read back, checked,
// hashed and logged by the backend (src-tauri/src/render_cmds.rs).
import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { SceneObject } from "../scene3d/model";
import type { Engine } from "../viewer3d/engine";
import type { Animation, Sample } from "./model";
import { drawOverlays, NO_OVERLAYS, type Overlays } from "./overlay";
import { showAt } from "./show";

type Model = Extract<SceneObject, { kind: "model" }>;

const SIZES: [number, number][] = [
  [1920, 1080],
  [1280, 720],
  [3840, 2160],
];

const LABELS: [keyof Overlays, string][] = [
  ["elapsed_time", "Elapsed time"],
  ["time_zero", "Time zero"],
  ["frame_number", "Frame number"],
  ["speeds", "Speed of each mover"],
  ["scale_bar", "Scale bar"],
  ["illustrative", '"Illustrative" on assumed motion'],
];

export function RenderSection({
  engine,
  a,
  models,
  sceneId,
  saving,
  onNotice,
  onDone,
}: {
  engine: () => Engine | null;
  a: Animation;
  models: Model[];
  sceneId: number;
  saving: boolean;
  onNotice: (m: string | null) => void;
  onDone: () => void;
}) {
  const [view, setView] = useState("");
  const [size, setSize] = useState(0);
  const [fps, setFps] = useState(30);
  const [overlays, setOverlays] = useState<Overlays>(NO_OVERLAYS);
  const [progress, setProgress] = useState<[number, number] | null>(null);
  const cancel = useRef(false);
  const v = a.views.find((x) => x.id === view);

  const run = async () => {
    const e = engine();
    if (!e || !v) return;
    const [w, h] = SIZES[size];
    // In-app test scripts can't answer the native dialog; they set the path it would return.
    const scripted = (globalThis as { __locusTestSavePath?: string }).__locusTestSavePath;
    const path =
      scripted ??
      (await saveDialog({
        defaultPath: `${v.name}.mp4`,
        filters: [{ name: "MP4 video", extensions: ["mp4"] }],
      }));
    if (!path) return;
    cancel.current = false;
    let started = false;
    try {
      const plan = await invoke<{ frames: number; labels: string[] }>("render_start", {
        request: {
          scene_id: sceneId,
          view: v.id,
          width: w,
          height: h,
          fps,
          from: a.from,
          to: a.to,
          overlays,
          path,
        },
      });
      started = true;
      // The motion and cameras at exactly each frame's time.
      const ev = await api.animationEvaluate(a, 1 / fps);
      const cams = ev.cameras.find(([id]) => id === v.id)?.[1];
      if (!cams) throw new Error("That view's mover isn't ready to play.");
      e.beginCapture(w, h, ["animation-now"]);
      try {
        for (let k = 0; k < plan.frames; k++) {
          if (cancel.current) throw new Error("Cancelled.");
          const t = a.from + k / fps;
          const states = ev.samples.map(([id, s]) => [id, s[k]] as [string, Sample]);
          showAt(e, a, models, states, v.id);
          const c = cams[k];
          const canvas = await e.captureFrame(c.eye, c.target, v.hfov_deg);
          drawOverlays(canvas, overlays, {
            t,
            frame: k,
            frames: plan.frames,
            timeZero: a.time_zero.event,
            view: v.name,
            hfovDeg: v.hfov_deg,
            movers: states.map(([id, s]) => [
              a.movers.find((m) => m.id === id)?.name ?? id,
              s.speed,
              s.assumed,
            ]),
            targetDistance: Math.hypot(...[0, 1, 2].map((i) => c.target[i] - c.eye[i])),
            labels: plan.labels,
          });
          const px = canvas.getContext("2d")!.getImageData(0, 0, w, h).data;
          await invoke("render_frame", new Uint8Array(px.buffer));
          setProgress([k + 1, plan.frames]);
        }
      } finally {
        e.endCapture();
      }
      const r = await invoke<{ id: number; record: { summary: string; sha256: string } }>(
        "render_finish",
        { name: `Render: ${v.name}` },
      );
      onNotice(
        `Rendered and checked: ${r.record.summary} SHA-256 ${r.record.sha256}; recorded as analysis ${r.id} in the audit log.`,
      );
    } catch (err) {
      if (started) await invoke("render_cancel").catch(() => {});
      onNotice(String(err));
    } finally {
      setProgress(null);
      onDone();
    }
  };

  return (
    <>
      <h4>Render to MP4</h4>
      <label>
        View
        <select value={view} onChange={(e) => setView(e.target.value)}>
          <option value="">Choose…</option>
          {a.views.map((x) => (
            <option key={x.id} value={x.id}>
              {x.name}
            </option>
          ))}
        </select>
      </label>
      <label>
        Size
        <select value={size} onChange={(e) => setSize(Number(e.target.value))}>
          {SIZES.map(([w, h], i) => (
            <option key={i} value={i}>
              {w} × {h}
            </option>
          ))}
        </select>
      </label>
      <label>
        Frames per second
        <select value={fps} onChange={(e) => setFps(Number(e.target.value))}>
          {[24, 25, 30, 60].map((x) => (
            <option key={x} value={x}>
              {x}
            </option>
          ))}
        </select>
      </label>
      <p className="muted">Overlays (off unless chosen):</p>
      {LABELS.map(([k, label]) => (
        <label key={k} className="inline">
          <input
            type="checkbox"
            checked={overlays[k]}
            onChange={(e) => setOverlays({ ...overlays, [k]: e.target.checked })}
          />
          {label}
        </label>
      ))}
      {v?.kind.kind === "driver" && (
        <p className="muted">
          Every frame from a driver view is labelled &quot;Vehicle interior (pillars, mirrors,
          dashboard) not shown&quot;, whatever the overlays.
        </p>
      )}
      {progress ? (
        <div className="row">
          <span className="anim-clock">
            Frame {progress[0]} of {progress[1]}
          </span>
          <button onClick={() => (cancel.current = true)}>Cancel</button>
        </div>
      ) : (
        <button
          className="primary"
          disabled={!v || saving}
          title={saving ? "Waiting for the scene to save" : undefined}
          onClick={() => void run()}
        >
          Render {a.from.toFixed(2)} to {a.to.toFixed(2)} s…
        </button>
      )}
    </>
  );
}
