// Bullet trajectory tool, in the 3D view's side panel. Pick defects (entry then exit on each
// surface, in the order the bullet travelled) or both ends of a probe rod; the result is
// previewed live from the backend (which re-resolves every pick), drawn over the scene, and
// saved as an audit-logged analysis record with a PDF report.
import { useUnsaved } from "../../unsaved";
import { PanelHeader } from "../../PanelHeader";
import { HelpButton } from "../../help/Help";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import {
  api,
  type AnalysisRecord,
  type EvidenceRecord,
  type Measured,
  type TrajectoryParameters,
  type TrajectoryRecord,
  type TrajectoryRequest,
  type TrajectoryRun,
} from "../../api";
import type { Engine } from "../../viewer3d/engine";
import type { PickHit } from "../../viewer3d/pointcloud";
import { trajectoryOverlay } from "./overlay";

type Kind = "entry" | "exit" | "rod";
interface Row {
  pick: PickHit;
  at: [number, number, number];
  kind: Kind;
  surface: string;
  sigmaMm: number;
  centre: "fitted" | "manual";
  reason: string;
  photo: number | null;
}

const DEFAULT_PARAMS: TrajectoryParameters = {
  cone_deg: 5,
  rod_play_deg: 0,
  band: [0.9, 1.8],
  floor_z: 0,
  max_range: 30,
  conventions: {
    surface: "level_perpendicular",
    reference: "project north (+y)",
    reference_deg: 0,
  },
};

const vertical = (m: Measured) =>
  `${Math.abs(m.value).toFixed(2)}° ± ${m.sigma.toFixed(2)}° ${m.value >= 0 ? "up" : "down"}`;
const horizontal = (m: Measured | null) =>
  m
    ? `${Math.abs(m.value).toFixed(1)}° ± ${m.sigma.toFixed(1)}° ${m.value >= 0 ? "right" : "left"} of perpendicular`
    : "horizontal surface";

const fmt = (m: { value: number; sigma: number }, digits = 2) =>
  `${m.value.toFixed(digits)}° ± ${m.sigma.toFixed(digits)}°`;

const num = (label: string, value: number, set: (v: number) => void, step = 0.1) => (
  <label>
    {label}
    <input
      type="number"
      value={Number(value.toFixed(4))}
      step={step}
      onChange={(e) => Number.isFinite(e.target.valueAsNumber) && set(e.target.valueAsNumber)}
    />
  </label>
);

export function TrajectoryPanel({
  engine,
  origin,
  requestPick,
  onNotice,
  photos,
}: {
  engine: () => Engine | null;
  origin: string;
  /** Image evidence a defect photo can be chosen from. */
  photos: EvidenceRecord[];
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  onNotice: (m: string | null) => void;
}) {
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"defects" | "rod">("defects");
  const [name, setName] = useState("Trajectory 1");
  const [rows, setRows] = useState<Row[]>([]);
  const [params, setParams] = useState(DEFAULT_PARAMS);
  const [planeRadius, setPlaneRadius] = useState(0.05);
  const [result, setResult] = useState<TrajectoryRun | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [shown, setShown] = useState<number | null>(null);
  const [caseNumber, setCaseNumber] = useState("");
  const [holeRadius, setHoleRadius] = useState(0.03);
  useUnsaved("Trajectory", open && rows.length > 0);

  useEffect(() => {
    api.analyses().then(setRecords, (e) => onNotice(String(e)));
    api.caseNumber().then(
      (c) => setCaseNumber(c ?? ""),
      (e) => onNotice(String(e)),
    );
  }, [onNotice]);

  const toRequest = (rs: Row[]): TrajectoryRequest => ({
    points: rs.map((r) => ({
      pick: r.pick,
      kind: r.kind,
      surface: r.surface,
      sigma: r.sigmaMm / 1000,
      centre: r.centre,
      override_reason: r.centre === "manual" ? r.reason : null,
      photo: r.photo,
    })),
    parameters: params,
    plane_radius: planeRadius,
    hole_radius: holeRadius,
  });
  const request = () => toRequest(rows);

  // Live preview while building.
  useEffect(() => {
    if (!open || rows.length < 2) return;
    let live = true;
    const req = toRequest(rows);
    api.trajectoryPreview(req).then(
      (r) => {
        if (!live) return;
        setResult(r);
        setFailure(null);
      },
      (e) => live && (setResult(null), setFailure(String(e))),
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- toRequest reads exactly these
  }, [open, rows, params, planeRadius, holeRadius]);

  // The preview applies only while building with at least two points.
  const building = open && rows.length >= 2;
  const preview = building ? result : null;
  const error = building ? failure : null;

  // What's drawn: the preview while building, else a shown record.
  const trajectories = records.filter((r): r is TrajectoryRecord => r.tool === "trajectory");
  const drawn = open ? preview : (trajectories.find((r) => r.id === shown)?.record ?? null);
  useEffect(() => {
    const e = engine();
    if (!e) return;
    e.setAnalysisOverlay("trajectory", drawn ? trajectoryOverlay(drawn, e.origin) : null);
  }, [drawn, engine, origin]);
  useEffect(() => () => engine()?.setAnalysisOverlay("trajectory", null), [engine]);

  const nextKind = (): Kind => {
    if (mode === "rod") return "rod";
    return rows.length % 2 === 0 ? "entry" : "exit";
  };
  const nextSurface = () => {
    if (mode === "rod") return "probe rod";
    const last = rows[rows.length - 1];
    if (last && last.kind === "entry") return last.surface;
    return `surface ${Math.floor(rows.length / 2) + 1}`;
  };
  const pick = () => {
    const kind = nextKind();
    requestPick(
      kind === "rod" ? "Click a point on the probe rod." : `Click the ${kind} defect's centre.`,
      async (hit) => {
        try {
          const r = await api.pickResolve(hit);
          setRows((rs) => [
            ...rs,
            {
              pick: hit,
              at: r.project,
              kind,
              surface: nextSurface(),
              sigmaMm: mode === "rod" ? 1 : 2,
              centre: "fitted",
              reason: "",
              photo: null,
            },
          ]);
        } catch (e) {
          onNotice(String(e));
        }
      },
    );
  };
  const setRow = (i: number, patch: Partial<Row>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  const saveRun = async () => {
    try {
      const rec = await api.trajectorySave(name, request(), null);
      setRecords(await api.analyses());
      setOpen(false);
      setRows([]);
      setShown(rec.id);
      onNotice(
        `Trajectory saved as analysis ${rec.id}: ${rec.record.summary} (recorded in the audit log).`,
      );
    } catch (e) {
      onNotice(String(e));
    }
  };

  return (
    <section className="panel-section trajectory">
      <PanelHeader>
        Bullet trajectory <HelpButton topic="trajectory" />
      </PanelHeader>
      {!open && (
        <button
          onClick={() => {
            setOpen(true);
            setRows([]);
            setName(`Trajectory ${trajectories.length + 1}`);
          }}
        >
          New trajectory…
        </button>
      )}
      <label>
        Case number
        <input
          value={caseNumber}
          onChange={(e) => setCaseNumber(e.target.value)}
          onBlur={() => api.caseNumberSet(caseNumber).catch((e) => onNotice(String(e)))}
        />
      </label>
      {open && (
        <div className="dg-built">
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <label>
            From
            <select
              value={mode}
              onChange={(e) => {
                setMode(e.target.value as "defects" | "rod");
                setRows([]);
                setParams((p) => ({ ...p, rod_play_deg: e.target.value === "rod" ? 2 : 0 }));
              }}
            >
              <option value="defects">Defects on surfaces</option>
              <option value="rod">A probe rod</option>
            </select>
          </label>
          <p className="muted">
            {mode === "defects"
              ? "Pick the defects in the order the bullet travelled: entry then exit on each surface, clicking inside each hole. Each centre is fitted from the hole's rim; use the picked point instead only with a reason."
              : "Pick two points well apart along the rod. Enter the rod's play in its hole."}
          </p>
          {rows.map((r, i) => (
            <fieldset key={i}>
              <legend>
                {i + 1}. {r.kind} ({r.at.map((v) => v.toFixed(3)).join(", ")})
              </legend>
              {r.kind !== "rod" && (
                <label>
                  Surface
                  <input
                    value={r.surface}
                    onChange={(e) => setRow(i, { surface: e.target.value })}
                  />
                </label>
              )}
              {r.kind !== "rod" && (
                <label>
                  Centre
                  <select
                    value={r.centre}
                    onChange={(e) => setRow(i, { centre: e.target.value as Row["centre"] })}
                  >
                    <option value="fitted">Fitted from the rim</option>
                    <option value="manual">The picked point (override)</option>
                  </select>
                </label>
              )}
              {r.centre === "manual" && r.kind !== "rod" && (
                <label>
                  Reason (in the report)
                  <input value={r.reason} onChange={(e) => setRow(i, { reason: e.target.value })} />
                </label>
              )}
              {(r.kind === "rod" || r.centre === "manual") &&
                num("1σ (mm)", r.sigmaMm, (sigmaMm) => sigmaMm > 0 && setRow(i, { sigmaMm }), 0.5)}
              {preview?.inputs[i]?.defect && (
                <div className="muted">
                  Centre ± {(preview.inputs[i].defect!.centre_sigma * 1000).toFixed(1)} mm from{" "}
                  {preview.inputs[i].defect!.rim_points} rim points; ellipse impact{" "}
                  {fmt(preview.inputs[i].defect!.impact, 1)}
                </div>
              )}
              {preview?.cross_checks
                ?.filter((c) => c.input === i && !c.agrees)
                .map((c) => (
                  <p key={c.input} className="error">
                    The hole's ellipse ({fmt(c.ellipse, 1)}) disagrees with the path (
                    {fmt(c.trajectory, 1)}) beyond their uncertainty.
                  </p>
                ))}
              {r.kind !== "rod" && (
                <label>
                  Photo
                  <select
                    value={r.photo ?? ""}
                    onChange={(e) =>
                      setRow(i, { photo: e.target.value ? Number(e.target.value) : null })
                    }
                  >
                    <option value="">None</option>
                    {photos.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.contents.images[0]?.name ?? p.original_path}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {preview?.line.residuals[i] !== undefined && (
                <div className="muted">
                  Residual {(preview.line.residuals[i] * 1000).toFixed(1)} mm
                </div>
              )}
              <button onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>Remove</button>
            </fieldset>
          ))}
          <button onClick={pick}>Pick the next point</button>

          <h3>Where a muzzle could have been</h3>
          {num(
            "Cone half-angle (°)",
            params.cone_deg,
            (cone_deg) => setParams({ ...params, cone_deg }),
            0.5,
          )}
          {num("Band from (m above floor)", params.band[0], (v) =>
            setParams({ ...params, band: [v, params.band[1]] }),
          )}
          {num("Band to (m above floor)", params.band[1], (v) =>
            setParams({ ...params, band: [params.band[0], v] }),
          )}
          {num(
            "Floor elevation (m)",
            params.floor_z,
            (floor_z) => setParams({ ...params, floor_z }),
            0.01,
          )}
          <button
            onClick={() =>
              requestPick("Click the floor.", async (hit) => {
                try {
                  const e = engine();
                  const s = await api.surfaceAt(hit, 0.1, e?.cameraProject().eye ?? [0, 0, 10]);
                  setParams((p) => ({ ...p, floor_z: s.point[2] }));
                } catch (err) {
                  onNotice(String(err));
                }
              })
            }
          >
            Pick the floor
          </button>
          {num(
            "Trace back up to (m)",
            params.max_range,
            (max_range) => max_range > 0 && setParams({ ...params, max_range }),
            1,
          )}
          {mode === "rod" &&
            num(
              "Rod play (°)",
              params.rod_play_deg,
              (rod_play_deg) => rod_play_deg >= 0 && setParams({ ...params, rod_play_deg }),
              0.5,
            )}
          {mode === "defects" &&
            num(
              "Surface plane radius (m)",
              planeRadius,
              (r) => r > 0 && r <= 0.5 && setPlaneRadius(r),
              0.01,
            )}
          {mode === "defects" &&
            num(
              "Hole search radius (m)",
              holeRadius,
              (r) => r > 0 && r <= 0.2 && setHoleRadius(r),
              0.005,
            )}

          <h3>Angle conventions</h3>
          <label>
            Surface angles
            <select
              value={params.conventions.surface}
              onChange={(e) =>
                setParams({
                  ...params,
                  conventions: { ...params.conventions, surface: e.target.value },
                })
              }
            >
              <option value="level_perpendicular">
                Vertical from level; horizontal from perpendicular
              </option>
              <option value="normal">Both from the surface normal</option>
            </select>
          </label>
          <label>
            Bearing from
            <input
              value={params.conventions.reference}
              onChange={(e) =>
                setParams({
                  ...params,
                  conventions: { ...params.conventions, reference: e.target.value },
                })
              }
            />
          </label>
          {num(
            "That axis, ° clockwise from project +y",
            params.conventions.reference_deg,
            (reference_deg) =>
              setParams({ ...params, conventions: { ...params.conventions, reference_deg } }),
            0.1,
          )}

          {error && <p className="error">{error}</p>}
          {preview && (
            <div className="trajectory-result">
              <div>
                Bearing <strong>{fmt(preview.scene_bearing ?? preview.line.bearing)}</strong> from{" "}
                {params.conventions.reference}
              </div>
              <div>
                Elevation <strong>{vertical(preview.line.elevation)}</strong>
              </div>
              <div className="muted">
                <span className="swatch measurement" /> Measurement uncertainty (95 %, computed){" "}
                {preview.line.cone.major_deg.toFixed(2)}° × {preview.line.cone.minor_deg.toFixed(2)}
                °
                <br />
                <span className="swatch examiner" /> Examiner-defined zone (±
                {params.cone_deg.toFixed(1)}°, analyst judgment)
                {preview.line.dof > 0
                  ? `; χ² ${preview.line.chi2.toFixed(2)} on ${preview.line.dof}`
                  : "; no redundancy"}
              </div>
              {preview.cone_narrower_than_fit && (
                <p className="error">
                  The computed 95 % cone ({preview.line.cone.major_deg.toFixed(1)}°) is wider than
                  the examiner-defined zone.
                </p>
              )}
              {preview.line.inflation > 1 && (
                <p className="error">
                  The points scatter more than their stated uncertainty; the uncertainty has been
                  widened {preview.line.inflation.toFixed(1)}×. Check for a misplaced pick or a
                  deflection.
                </p>
              )}
              {preview.surfaces.map((s) => (
                <div key={s.surface} className="muted">
                  {s.surface}: impact {fmt(s.angles.impact, 1)},{" "}
                  {params.conventions.surface === "level_perpendicular" && s.level
                    ? `${vertical(s.level.vertical)}, ${horizontal(s.level.horizontal)}`
                    : `horizontal ${fmt(s.angles.horizontal, 1)}, vertical ${fmt(s.angles.vertical, 1)}`}
                </div>
              ))}
              {preview.band.centre && (
                <div className="muted">
                  In the band from {preview.band.centre[0][0].toFixed(2)} to{" "}
                  {preview.band.centre[0][1].toFixed(2)} m back from the first point.
                </div>
              )}
            </div>
          )}
          <div className="buttons">
            <button onClick={() => setOpen(false)}>Cancel</button>
            <button className="primary" disabled={!preview} onClick={() => void saveRun()}>
              Save analysis
            </button>
          </div>
        </div>
      )}
      {trajectories.map((r) => (
        <div key={r.id} className={`dg-object${shown === r.id ? " active" : ""}`}>
          <button className="link" onClick={() => setShown(shown === r.id ? null : r.id)}>
            {r.name}
            {r.withdrawn ? " (withdrawn)" : ""}
          </button>
          <div className="muted">{r.record.summary}</div>
          <div className="buttons">
            <button
              onClick={async () => {
                const path = await saveDialog({
                  defaultPath: `${r.name}.pdf`,
                  filters: [{ name: "PDF", extensions: ["pdf"] }],
                });
                if (!path) return;
                try {
                  const sha = await api.analysisReport(r.id, path);
                  onNotice(`Report saved to ${path} (SHA-256 ${sha}; recorded in the audit log).`);
                } catch (e) {
                  onNotice(String(e));
                }
              }}
            >
              Report PDF…
            </button>
            {!r.withdrawn && (
              <button
                onClick={async () => {
                  const reason = window.prompt(
                    "Why is this analysis withdrawn? (recorded in the audit log)",
                  );
                  if (!reason?.trim()) return;
                  try {
                    setRecords(await api.analysisWithdraw(r.id, reason));
                  } catch (e) {
                    onNotice(String(e));
                  }
                }}
              >
                Withdraw…
              </button>
            )}
          </div>
        </div>
      ))}
    </section>
  );
}
