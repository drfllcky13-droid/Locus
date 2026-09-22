import { useState } from "react";
import { api, type CleanupRecord, type CleanupRequest, type Region, type StateView } from "../api";
import { formatCount } from "../format";
import { formatCoordinate, formatMeasurement } from "./measureFormat";
import type { SceneData } from "./pointcloud";
import type { Tool, ViewSettings } from "./Viewport";

export interface BuildStatus {
  name: string;
  progress: { stage: string; done: number; total: number } | null;
  error: string | null;
  done: boolean;
}

const TOOLS: { id: Tool; label: string }[] = [
  { id: "orbit", label: "Navigate" },
  { id: "distance", label: "Distance" },
  { id: "angle", label: "Angle" },
  { id: "area", label: "Area" },
  { id: "height", label: "Height" },
  { id: "lasso", label: "Lasso delete" },
];

const CLEANUP_LABEL: Record<CleanupRecord["kind"], string> = {
  box_delete: "Box delete",
  lasso_delete: "Lasso delete",
  outliers: "Outlier removal",
  voxel: "Voxel downsample",
};

export function ScenePanel(props: {
  scene: SceneData | null;
  state: StateView | null;
  builds: Record<string, BuildStatus>;
  tool: Tool;
  setTool: (t: Tool) => void;
  settings: ViewSettings;
  setSettings: (f: (s: ViewSettings) => ViewSettings) => void;
  region: () => Region | null;
  runCleanup: (r: CleanupRequest) => Promise<void>;
  setState: (s: StateView) => void;
  onNotice: (m: string | null) => void;
}) {
  const { scene, state, settings, setSettings } = props;
  const [k, setK] = useState(8);
  const [stdMult, setStdMult] = useState(2);
  const [voxel, setVoxel] = useState(0.01);
  const [sigmaMm, setSigmaMm] = useState<string>("");
  const set = (patch: Partial<ViewSettings>) => setSettings((s) => ({ ...s, ...patch }));
  const building = Object.entries(props.builds).filter(([, b]) => !b.done || b.error);

  return (
    <aside className="scene-panel">
      <h2>Point clouds</h2>
      {scene?.scans.length ? (
        <ul className="scans">
          {scene.scans.map((s) => (
            <li key={s.key}>
              {s.name} <span className="muted">{formatCount(s.points, "point")}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">No point clouds yet. Import a scan to build one.</p>
      )}
      {building.map(([key, b]) => (
        <p key={key} className={b.error ? "error" : "muted"}>
          {b.error
            ? `${b.name}: ${b.error}`
            : `Building ${b.name}: ${b.progress?.stage ?? "starting"} ${
                b.progress?.total
                  ? Math.round((b.progress.done / b.progress.total) * 100) + "%"
                  : ""
              }`}
        </p>
      ))}

      <h2>Tools</h2>
      <div className="toolbar">
        {TOOLS.map((t) => (
          <button
            key={t.id}
            className={props.tool === t.id ? "primary" : ""}
            onClick={() => props.setTool(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      <h2>Display</h2>
      <label className="inline">
        Colour
        <select
          value={settings.color}
          onChange={(e) => set({ color: e.target.value as ViewSettings["color"] })}
        >
          <option value="rgb">RGB</option>
          <option value="intensity">Intensity</option>
          <option value="elevation">Elevation</option>
        </select>
      </label>
      <label className="inline">
        Point size ×{settings.pointSize.toFixed(1)}
        <input
          type="range"
          min={0.3}
          max={3}
          step={0.1}
          value={settings.pointSize}
          onChange={(e) => set({ pointSize: Number(e.target.value) })}
        />
      </label>
      <label className="inline">
        <input
          type="checkbox"
          checked={settings.edl}
          onChange={(e) => set({ edl: e.target.checked })}
        />{" "}
        Eye-dome lighting
      </label>

      <h2>Clipping</h2>
      <label className="inline">
        Box
        <select
          value={settings.clip}
          onChange={(e) => set({ clip: e.target.value as ViewSettings["clip"] })}
        >
          <option value="off">Off</option>
          <option value="inside">Show inside</option>
          <option value="outside">Show outside</option>
        </select>
      </label>
      {settings.clip !== "off" && (
        <p className="muted">Drag the box handles. T moves, S resizes.</p>
      )}
      <label className="inline">
        <input
          type="checkbox"
          checked={settings.plane.on}
          onChange={(e) => set({ plane: { ...settings.plane, on: e.target.checked } })}
        />{" "}
        Plane
        <select
          value={settings.plane.axis}
          onChange={(e) =>
            set({ plane: { ...settings.plane, axis: Number(e.target.value) as 0 | 1 | 2 } })
          }
        >
          <option value={0}>X</option>
          <option value={1}>Y</option>
          <option value={2}>Z</option>
        </select>
        <input
          type="number"
          step={0.1}
          value={settings.plane.offset}
          aria-label="Plane position in meters"
          onChange={(e) => set({ plane: { ...settings.plane, offset: Number(e.target.value) } })}
        />
        m
        <label>
          <input
            type="checkbox"
            checked={settings.plane.flip}
            onChange={(e) => set({ plane: { ...settings.plane, flip: e.target.checked } })}
          />{" "}
          flip
        </label>
      </label>

      <h2>Cleanup</h2>
      <p className="muted">
        Cleanup never changes evidence. Each operation can be undone (Ctrl+Z) and is logged.
      </p>
      <div className="toolbar">
        <button
          disabled={settings.clip === "off"}
          title="Turn on the clip box first"
          onClick={() => {
            const region = props.region();
            if (region) void props.runCleanup({ kind: "box_delete", region });
          }}
        >
          Delete inside box
        </button>
      </div>
      <label className="inline">
        Outliers: {k} neighbours, beyond
        <input
          type="number"
          min={1}
          max={64}
          value={k}
          aria-label="Neighbours"
          onChange={(e) => setK(Number(e.target.value))}
        />
        <input
          type="number"
          min={0.5}
          step={0.5}
          value={stdMult}
          aria-label="Standard deviations"
          onChange={(e) => setStdMult(Number(e.target.value))}
        />
        σ
        <button
          onClick={() =>
            void props.runCleanup({
              kind: "outliers",
              k,
              std_mult: stdMult,
              region: props.region(),
            })
          }
        >
          Remove
        </button>
      </label>
      <label className="inline">
        Voxel downsample
        <input
          type="number"
          min={0.001}
          step={0.005}
          value={voxel}
          aria-label="Voxel size in meters"
          onChange={(e) => setVoxel(Number(e.target.value))}
        />
        m
        <button
          onClick={() =>
            void props.runCleanup({ kind: "voxel", size: voxel, region: props.region() })
          }
        >
          Apply
        </button>
      </label>
      <p className="muted">
        Outlier removal and downsampling work inside the clip box when it is on, otherwise on
        everything.
      </p>
      {state?.cleanups.length ? (
        <ul className="cleanups">
          {state.cleanups.map((c) => (
            <li key={c.id}>
              <label>
                <input
                  type="checkbox"
                  checked={c.active}
                  onChange={async (e) => {
                    try {
                      props.setState(await api.cleanupSetActive(c.id, e.target.checked));
                    } catch (err) {
                      props.onNotice(String(err));
                    }
                  }}
                />
                #{c.id} {CLEANUP_LABEL[c.kind]}:{" "}
                {formatCount(
                  c.scans.reduce((n, s) => n + s.removed, 0),
                  "point",
                )}
                {c.active ? "" : " (undone)"}
              </label>
            </li>
          ))}
        </ul>
      ) : null}

      <h2>Measurements</h2>
      <label className="inline">
        Point uncertainty (1σ)
        <input
          type="number"
          min={0.1}
          step={0.1}
          placeholder={state ? (state.point_sigma_m * 1000).toFixed(1) : ""}
          value={sigmaMm}
          aria-label="Point uncertainty in millimetres"
          onChange={(e) => setSigmaMm(e.target.value)}
        />
        mm
        <button
          disabled={!sigmaMm}
          onClick={async () => {
            try {
              props.setState(await api.setPointSigma(Number(sigmaMm) / 1000));
              setSigmaMm("");
            } catch (e) {
              props.onNotice(String(e));
            }
          }}
        >
          Set
        </button>
      </label>
      {state?.measurements.length ? (
        <ul className="measurements">
          {state.measurements.map((m) => {
            const f = formatMeasurement(m);
            return (
              <li key={m.id}>
                <div>
                  <strong>
                    #{m.id} {m.kind}
                  </strong>{" "}
                  {f.label}
                  <button
                    className="link"
                    aria-label={`Delete measurement ${m.id}`}
                    onClick={async () => props.setState(await api.measurementDelete(m.id))}
                  >
                    ×
                  </button>
                </div>
                <div className="muted">{f.detail}</div>
                <details>
                  <summary className="muted">{m.points.length} points</summary>
                  {m.points.map((p, i) => (
                    <div key={i} className="coord">
                      {formatCoordinate(p.project)}{" "}
                      <span className="muted">
                        (scan {p.scan}, record {p.index})
                      </span>
                    </div>
                  ))}
                </details>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="muted">No measurements yet.</p>
      )}
    </aside>
  );
}
