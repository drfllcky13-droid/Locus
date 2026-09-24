// Witness perspective, in the 3D view's side panel: an eye at a stated height above a picked
// floor point, looking toward a picked point, with lines of sight to picked targets tested
// against the scan (clear in green, blocked in red, drawn in the 3D view). The view can be
// set to the eye. A run is saved as an audit-logged analysis with a PDF report.
import { HelpButton } from "../../help/Help";
import { useEffect, useState } from "react";
import * as THREE from "three";
import {
  api,
  type AnalysisRecord,
  type WitnessRecord,
  type WitnessRequest,
  type WitnessRun,
} from "../../api";
import type { Engine } from "../../viewer3d/engine";
import type { PickHit } from "../../viewer3d/pointcloud";
import { RecordButtons } from "./CameraPanel";

type V3 = [number, number, number];

function witnessOverlay(run: WitnessRun, origin: V3): THREE.Group {
  const g = new THREE.Group();
  g.name = "witness";
  const rel = (p: V3) => new THREE.Vector3(p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]);
  for (const s of run.sights) {
    const line = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([rel(s.from), rel(s.to)]),
      new THREE.LineBasicMaterial({ color: s.clear ? 0x30d158 : 0xff453a, depthTest: false }),
    );
    line.renderOrder = 10;
    g.add(line);
    if (s.first) {
      const dot = new THREE.Mesh(
        new THREE.SphereGeometry(0.03, 12, 8),
        new THREE.MeshBasicMaterial({ color: 0xff453a, depthTest: false }),
      );
      dot.position.copy(rel(s.first));
      dot.renderOrder = 11;
      g.add(dot);
    }
  }
  const eye = new THREE.Mesh(
    new THREE.SphereGeometry(0.04, 12, 8),
    new THREE.MeshBasicMaterial({ color: 0x3fa9ff, depthTest: false }),
  );
  eye.position.copy(rel(run.eye));
  eye.renderOrder = 11;
  g.add(eye);
  const post = new THREE.Line(
    new THREE.BufferGeometry().setFromPoints([rel(run.floor_point), rel(run.eye)]),
    new THREE.LineDashedMaterial({
      color: 0x3fa9ff,
      dashSize: 0.05,
      gapSize: 0.04,
      depthTest: false,
    }),
  );
  post.computeLineDistances();
  g.add(post);
  return g;
}

export function WitnessPanel({
  engine,
  origin,
  requestPick,
  onNotice,
}: {
  engine: () => Engine | null;
  origin: string;
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  onNotice: (m: string | null) => void;
}) {
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("Witness view 1");
  const [floor, setFloor] = useState<PickHit | null>(null);
  const [eyeHeight, setEyeHeight] = useState("");
  const [lookAt, setLookAt] = useState<V3 | null>(null);
  const [fov, setFov] = useState(60);
  const [targets, setTargets] = useState<{ label: string; pick: PickHit }[]>([]);
  const [result, setResult] = useState<WitnessRun | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [shown, setShown] = useState<number | null>(null);

  useEffect(() => {
    api.analyses().then(setRecords, (e) => onNotice(String(e)));
  }, [onNotice]);

  const h = Number(eyeHeight);
  const req: WitnessRequest | null =
    floor && lookAt && h > 0
      ? { floor, eye_height: h, look_at: lookAt, fov_deg: fov, targets }
      : null;
  const reqKey = JSON.stringify(req);
  useEffect(() => {
    if (!open || !req) return;
    let live = true;
    api.witnessPreview(req).then(
      (r) => live && (setResult(r), setFailure(null)),
      (e) => live && (setResult(null), setFailure(String(e))),
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reqKey stands for req
  }, [open, reqKey]);
  const preview = open && req ? result : null;
  const witnesses = records.filter((r): r is WitnessRecord => r.tool === "witness");
  const drawn = open ? preview : (witnesses.find((r) => r.id === shown)?.record ?? null);

  useEffect(() => {
    const e = engine();
    if (e) e.setAnalysisOverlay("witness", drawn ? witnessOverlay(drawn, e.origin) : null);
  }, [drawn, engine, origin]);
  useEffect(() => () => engine()?.setAnalysisOverlay("witness", null), [engine]);

  const resolved = async (hit: PickHit): Promise<V3 | null> => {
    try {
      return (await api.pickResolve(hit)).project as V3;
    } catch (e) {
      onNotice(String(e));
      return null;
    }
  };

  const save = async () => {
    if (!req) return;
    try {
      const rec = await api.witnessSave(name, req, null);
      setRecords(await api.analyses());
      setOpen(false);
      setShown(rec.id);
      onNotice(`Saved as analysis ${rec.id}: ${rec.record.summary} (recorded in the audit log).`);
    } catch (e) {
      onNotice(String(e));
    }
  };

  return (
    <section className="panel-section witness">
      <h3 className="with-help">
        Witness perspective <HelpButton topic="camera-height" />
      </h3>
      {!open && (
        <button
          onClick={() => {
            setOpen(true);
            setFloor(null);
            setLookAt(null);
            setTargets([]);
            setEyeHeight("");
            setName(`Witness view ${witnesses.length + 1}`);
          }}
        >
          New witness view…
        </button>
      )}
      {open && (
        <div className="dg-built">
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <button onClick={() => requestPick("Click the floor where the witness stood.", setFloor)}>
            {floor ? "Standing point ✓ (pick again)" : "Pick where the witness stood"}
          </button>
          <label>
            Eye height above that point (m, as stated)
            <input
              className="narrow"
              inputMode="decimal"
              value={eyeHeight}
              onChange={(e) => setEyeHeight(e.target.value)}
            />
          </label>
          <button
            onClick={() =>
              requestPick("Click the point the witness was looking toward.", async (hit) => {
                const p = await resolved(hit);
                if (p) setLookAt(p);
              })
            }
          >
            {lookAt ? "Looking toward ✓ (pick again)" : "Pick where they were looking"}
          </button>
          <label>
            Field of view (° horizontal)
            <input
              type="number"
              step={5}
              value={fov}
              onChange={(e) => e.target.valueAsNumber > 0 && setFov(e.target.valueAsNumber)}
            />
          </label>
          <h3>Targets</h3>
          {targets.map((t, i) => {
            const s = preview?.sights[i];
            return (
              <div key={i} className="buttons">
                <input
                  value={t.label}
                  onChange={(e) =>
                    setTargets((ts) =>
                      ts.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)),
                    )
                  }
                />
                {s && (
                  <span className={s.clear ? "" : "error"}>
                    {s.clear
                      ? `clear (${s.length.toFixed(2)} m)`
                      : `blocked at ${s.first_distance?.toFixed(2)} m`}
                  </span>
                )}
                <button onClick={() => setTargets((ts) => ts.filter((_, j) => j !== i))}>
                  Remove
                </button>
              </div>
            );
          })}
          <button
            onClick={() =>
              requestPick("Click a target to test the line of sight to.", (pick) =>
                setTargets((ts) => [...ts, { label: `Target ${ts.length + 1}`, pick }]),
              )
            }
          >
            Add target
          </button>
          {failure && req && <p className="error">{failure}</p>}
          {preview && <p className="muted">{preview.summary}</p>}
          <div className="buttons">
            <button
              disabled={!preview}
              onClick={() =>
                preview && engine()?.setView(preview.eye, preview.look_at, preview.fov_deg)
              }
            >
              View from the eye
            </button>
            <button
              onClick={() => {
                const e = engine();
                if (!e) return;
                const v = e.cameraProject();
                e.setView(v.eye, v.target, null);
              }}
            >
              Normal field of view
            </button>
            <button onClick={() => setOpen(false)}>Cancel</button>
            <button className="primary" disabled={!preview} onClick={() => void save()}>
              Save analysis
            </button>
          </div>
        </div>
      )}
      {witnesses.map((r) => (
        <div key={r.id} className={`dg-object${shown === r.id ? " active" : ""}`}>
          <button className="link" onClick={() => setShown(shown === r.id ? null : r.id)}>
            {r.name}
            {r.withdrawn ? " (withdrawn)" : ""}
          </button>
          <div className="muted">{r.record.summary}</div>
          {shown === r.id && (
            <button
              onClick={() => engine()?.setView(r.record.eye, r.record.look_at, r.record.fov_deg)}
            >
              View from the eye
            </button>
          )}
          <RecordButtons
            id={r.id}
            name={r.name}
            withdrawn={!!r.withdrawn}
            setRecords={setRecords}
            onNotice={onNotice}
          />
        </div>
      ))}
    </section>
  );
}
