// Camera matching and subject height, in the 3D view's side panel. Choose the photo or CCTV
// frame, pair points in it with points on the scan (a pixel, then the same point on the
// cloud), and the backend solves the camera from the stored data. Scan points and the pairs'
// residuals are drawn over the photo; the 3D view can look through the solved camera with the
// photo over the scene. For each subject, mark the point between the feet and the top of the
// head: the height comes by reverse projection, with a person model of that height drawn over
// the photo to match by eye. A run is saved as an audit-logged analysis with a PDF report.
import { useUnsaved } from "../../unsaved";
import { HelpButton } from "../../help/Help";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import * as THREE from "three";
import {
  api,
  type AnalysisRecord,
  type CameraParameters,
  type CameraRecord,
  type CameraRequest,
  type CameraRun,
  type LensModel,
  type SolvedCamera,
} from "../../api";
import { useUnderlayUrl } from "../../diagram2d/UnderlayPanel";
import { build, POSES } from "../../scene3d/library";
import type { Engine } from "../../viewer3d/engine";
import type { PickHit } from "../../viewer3d/pointcloud";
import { PhotoEditor } from "../bloodstain/PhotoEditor";
import { maxRadius, project } from "./model";

type P2 = [number, number];
type V3 = [number, number, number];
type Photo = Awaited<ReturnType<typeof api.underlayImages>>[number];
type Mode = "pairs" | "feet" | "head";
type PoseName = keyof typeof POSES;

interface Subject {
  label: string;
  feet: P2 | null;
  head: P2 | null;
  /** The head point was set from a matched person model of this stature. */
  matched: number | null;
  /** The frame the points are marked on, when not the camera's photo (same fixed camera). */
  frame: Photo | null;
  /** The person model drawn over the photo: stature (m, as typed), pose and facing. */
  model: string;
  pose: PoseName;
  heading: number;
}

const DEFAULT_PARAMS: CameraParameters = {
  model: "auto",
  pick_sigma_px: 1,
  point_sigma: 0,
  floor_z: 0,
  draws: 1000,
  seed: 1,
};

const LENSES: [LensModel, string][] = [
  ["auto", "Automatic (leave-one-out)"],
  ["pinhole", "Focal length only"],
  ["radial1", "Focal length, k1"],
  ["radial2", "Focal length, k1, k2"],
  ["full", "Full (principal point, k1–k3, p1, p2)"],
];

/** A person model's triangles in the project frame, standing at `feet` facing `heading`
 * (clockwise from +y). */
export function personTriangles(height: number, pose: PoseName, heading: number, feet: V3): V3[][] {
  const h = (heading * Math.PI) / 180;
  const fwd = [Math.sin(h), Math.cos(h)];
  const left = [-Math.cos(h), Math.sin(h)];
  const out: V3[][] = [];
  for (const m of Object.values(build({ type: "person", height, pose: POSES[pose] }))) {
    if (!m) continue;
    const v = (i: number): V3 => {
      const [x, y, z] = [m.positions[3 * i], m.positions[3 * i + 1], m.positions[3 * i + 2]];
      return [feet[0] + x * fwd[0] + y * left[0], feet[1] + x * fwd[1] + y * left[1], feet[2] + z];
    };
    for (let t = 0; t < m.indices.length; t += 3)
      out.push([v(m.indices[t]), v(m.indices[t + 1]), v(m.indices[t + 2])]);
  }
  return out;
}

function request(
  photo: Photo | null,
  pairs: { px: P2; pick: PickHit | null }[],
  subjects: Subject[],
  parameters: CameraParameters,
): CameraRequest | null {
  const done = pairs.filter((p) => p.pick);
  if (!photo || done.length < 6) return null;
  return {
    photo: photo.evidence_id,
    size: [photo.width, photo.height],
    pairs: done.map((p) => ({ px: p.px, pick: p.pick! })),
    parameters,
    subjects: subjects
      .filter((s) => s.feet && s.head)
      .map((s) => ({
        label: s.label,
        feet_px: s.feet!,
        head_px: s.head!,
        matched_model: s.matched,
        frame: s.frame?.evidence_id ?? null,
      })),
  };
}

/** Load a photo for the editor. */
function useImage(url: string | null) {
  const [loaded, setLoaded] = useState<{ url: string; image: HTMLImageElement } | null>(null);
  useEffect(() => {
    if (!url) return;
    const im = new Image();
    im.onload = () => setLoaded({ url, image: im });
    im.src = url;
  }, [url]);
  return loaded && loaded.url === url ? loaded.image : null;
}

/** The 3D overlay: each subject's person model where it stands, and the camera. */
function cameraOverlay(run: CameraRun, subjects: Subject[], origin: V3): THREE.Group {
  const g = new THREE.Group();
  g.name = "camera";
  const rel = (p: V3) => new THREE.Vector3(p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]);
  run.heights.forEach((h, k) => {
    const s = subjects.filter((x) => x.feet && x.head)[k];
    const height = Number(s?.model) > 0 ? Number(s.model) : h.height.value;
    const tris = personTriangles(height, s?.pose ?? "standing", s?.heading ?? 0, h.feet);
    const pos = tris.flatMap((t) => t.flatMap((p) => rel(p).toArray()));
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    const mesh = new THREE.Mesh(
      geo,
      new THREE.MeshBasicMaterial({
        color: 0x30d158,
        transparent: true,
        opacity: 0.45,
        side: THREE.DoubleSide,
        depthWrite: false,
      }),
    );
    mesh.renderOrder = 8;
    g.add(mesh);
  });
  // The camera: its centre and a small frustum toward what it sees.
  const c = run.solve.camera;
  const at = (u: number, v: number) => {
    const [a, b] = [(u - c.cx) / c.f, (v - c.cy) / c.f];
    const d = [0, 1, 2].map((i) => c.rotation[0][i] * a + c.rotation[1][i] * b + c.rotation[2][i]);
    return rel([0, 1, 2].map((i) => c.position[i] + 0.4 * d[i]) as V3);
  };
  const eye = rel(c.position);
  const corners = [at(0, 0), at(c.size[0], 0), at(c.size[0], c.size[1]), at(0, c.size[1])];
  const pts = corners.flatMap((q, i) => [eye, q, q, corners[(i + 1) % 4]]);
  const lines = new THREE.LineSegments(
    new THREE.BufferGeometry().setFromPoints(pts),
    new THREE.LineBasicMaterial({ color: 0x3fa9ff, depthTest: false }),
  );
  lines.renderOrder = 10;
  g.add(lines);
  return g;
}

export function CameraPanel({
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
  const [photos, setPhotos] = useState<Photo[]>([]);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("Camera match 1");
  const [photo, setPhoto] = useState<Photo | null>(null);
  const [pairs, setPairs] = useState<{ px: P2; pick: PickHit | null }[]>([]);
  const [subjects, setSubjects] = useState<Subject[]>([]);
  const [active, setActive] = useState(0);
  const [params, setParams] = useState(DEFAULT_PARAMS);
  const [editing, setEditing] = useState(false);
  const [mode, setMode] = useState<Mode>("pairs");
  const [result, setResult] = useState<CameraRun | null>(null);
  useUnsaved("Camera match", open && pairs.length > 0);
  const [failure, setFailure] = useState<string | null>(null);
  const [points, setPoints] = useState<V3[]>([]);
  const [look, setLook] = useState(false);
  const [opacity, setOpacity] = useState(0.5);
  const [shown, setShown] = useState<number | null>(null);

  useEffect(() => {
    api.analyses().then(setRecords, (e) => onNotice(String(e)));
  }, [onNotice]);

  const req = request(photo, pairs, subjects, params);
  const reqKey = JSON.stringify(req);
  useEffect(() => {
    if (!open || !req) return;
    let live = true;
    const t = setTimeout(() => {
      api.cameraPreview(req).then(
        (r) => {
          if (!live) return;
          setResult(r);
          setFailure(null);
          // Scan points to draw over the photo, as the solved camera sees them.
          setPoints(engine()?.samplePoints(20000) ?? []);
        },
        (e) => live && (setResult(null), setFailure(String(e))),
      );
    }, 300);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reqKey stands for req
  }, [open, reqKey]);
  const preview = open && req ? result : null;
  const error = open && req ? failure : null;

  const cameras = records.filter((r): r is CameraRecord => r.tool === "camera");
  const drawn = open ? preview : (cameras.find((r) => r.id === shown)?.record ?? null);
  const photoUrl = useUnderlayUrl(
    drawn?.photo?.file ?? photo?.file ?? "",
    drawn?.photo?.sha256 ?? photo?.sha256 ?? "",
  );
  // The editor shows the camera's photo, or the active subject's own frame while marking it.
  const onFrame = mode !== "pairs" ? (subjects[active]?.frame ?? null) : null;
  const shownPhoto = onFrame ?? photo;
  const editorUrl = useUnderlayUrl(shownPhoto?.file ?? "", shownPhoto?.sha256 ?? "");
  const image = useImage(editorUrl);

  // The 3D overlay, and looking through the camera.
  useEffect(() => {
    const e = engine();
    if (!e) return;
    e.setAnalysisOverlay("camera", drawn ? cameraOverlay(drawn, subjects, e.origin) : null);
  }, [drawn, subjects, engine, origin]);
  useEffect(() => {
    const e = engine();
    if (!e) return;
    e.setCameraMatch(look && drawn ? { camera: drawn.solve.camera, photoUrl, opacity } : null);
  }, [look, drawn, photoUrl, opacity, engine]);
  useEffect(
    () => () => {
      const e = engine();
      e?.setAnalysisOverlay("camera", null);
      e?.setCameraMatch(null);
    },
    [engine],
  );

  const patchSubject = useCallback((i: number, p: Partial<Subject>) => {
    setSubjects((ss) => ss.map((s, j) => (j === i ? { ...s, ...p } : s)));
  }, []);

  const click = (px: P2) => {
    const shown = shownPhoto;
    if (!shown || px[0] < 0 || px[1] < 0 || px[0] > shown.width || px[1] > shown.height) return;
    if (mode === "pairs") {
      const i = pairs.length;
      setPairs((ps) => [...ps, { px, pick: null }]);
      requestPick(`Click point ${i + 1} on the scan: the same place as in the photo.`, (hit) =>
        setPairs((ps) => ps.map((p, j) => (j === i ? { ...p, pick: hit } : p))),
      );
    } else if (subjects[active]) {
      patchSubject(active, mode === "feet" ? { feet: px } : { head: px, matched: null });
    }
  };

  // The person model of each subject, projected into the photo.
  const cam: SolvedCamera | null = preview?.solve.camera ?? null;
  const models = useMemo(() => {
    if (!cam || !preview) return [];
    const rmax = maxRadius(cam);
    return preview.heights.map((h, k) => {
      const s = subjects.filter((x) => x.feet && x.head)[k];
      const height = Number(s?.model) > 0 ? Number(s.model) : h.height.value;
      return personTriangles(height, s?.pose ?? "standing", s?.heading ?? 0, h.feet)
        .map((t) => t.map((p) => project(cam, p, rmax)))
        .filter((t): t is P2[] => t.every((p) => p !== null));
    });
  }, [cam, preview, subjects]);

  const draw = useCallback(
    (ctx: CanvasRenderingContext2D, at: (p: P2) => P2) => {
      if (!cam || !preview) return;
      const rmax = maxRadius(cam);
      ctx.fillStyle = "rgba(63, 169, 255, 0.55)";
      for (const p of points) {
        const q = project(cam, p, rmax);
        if (!q) continue;
        const [x, y] = at(q);
        ctx.fillRect(x - 0.75, y - 0.75, 1.5, 1.5);
      }
      ctx.fillStyle = "rgba(48, 209, 88, 0.3)";
      for (const tris of models)
        for (const t of tris) {
          ctx.beginPath();
          t.forEach((p, i) => {
            const [x, y] = at(p);
            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          });
          ctx.fill();
        }
      // Residuals, ten times longer (on the camera's photo only).
      ctx.strokeStyle = "#ff453a";
      if (onFrame) return;
      ctx.lineWidth = 2;
      const done = pairs.filter((p) => p.pick);
      preview.solve.residuals.forEach((d, i) => {
        const p = done[i]?.px;
        if (!p) return;
        const [a, b] = [at(p), at([p[0] + 10 * d[0], p[1] + 10 * d[1]])];
        ctx.beginPath();
        ctx.moveTo(a[0], a[1]);
        ctx.lineTo(b[0], b[1]);
        ctx.stroke();
      });
      ctx.fillStyle = "#30d158";
      ctx.font = "12px sans-serif";
      subjects.forEach((s) => {
        for (const [p, t] of [
          [s.feet, "feet"],
          [s.head, "head"],
        ] as const) {
          if (!p) continue;
          const [x, y] = at(p);
          ctx.fillRect(x - 3, y - 3, 6, 6);
          ctx.fillText(`${s.label} ${t}`, x + 6, y - 6);
        }
      });
    },
    [cam, preview, points, models, pairs, subjects, onFrame],
  );

  /** Subject i's result: the heights come in the order of the subjects with both points. */
  const heightOf = (i: number) =>
    subjects[i]?.feet && subjects[i]?.head
      ? preview?.heights[subjects.slice(0, i).filter((x) => x.feet && x.head).length]
      : undefined;

  const matchModel = (i: number) => {
    const s = subjects[i];
    const h = heightOf(i);
    const stature = Number(s.model);
    if (!cam || !h || !(stature > 0)) return;
    const top = project(cam, [h.feet[0], h.feet[1], h.feet[2] + stature]);
    if (top) patchSubject(i, { head: top, matched: stature });
  };

  const saveRun = async () => {
    if (!req) return;
    try {
      const rec = await api.cameraSave(name, req, null);
      setRecords(await api.analyses());
      setOpen(false);
      setEditing(false);
      setShown(rec.id);
      onNotice(`Saved as analysis ${rec.id}: ${rec.record.summary} (recorded in the audit log).`);
    } catch (e) {
      onNotice(String(e));
    }
  };

  const done = pairs.filter((p) => p.pick).length;
  const fmt = (v: number, s: number, d = 3) => `${v.toFixed(d)} ± ${s.toFixed(d)}`;
  return (
    <section className="panel-section camera-match">
      <h3 className="with-help">
        Camera matching and height <HelpButton topic="camera-height" />
      </h3>
      {!open && (
        <button
          onClick={() => {
            setOpen(true);
            setPairs([]);
            setSubjects([]);
            setPhoto(null);
            setName(`Camera match ${cameras.length + 1}`);
            api.underlayImages().then(setPhotos, (e) => onNotice(String(e)));
          }}
        >
          New camera match…
        </button>
      )}
      {open && (
        <div className="dg-built">
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <label>
            Photo or frame
            <select
              value={photo?.evidence_id ?? ""}
              onChange={(e) => {
                setPhoto(photos.find((p) => p.evidence_id === Number(e.target.value)) ?? null);
                setPairs([]);
              }}
            >
              <option value="">Choose an image…</option>
              {photos.map((p) => (
                <option key={p.evidence_id} value={p.evidence_id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          {photo && (
            <button onClick={() => setEditing(!editing)}>
              {editing ? "Close photo" : "Edit on photo…"}
            </button>
          )}
          <p className="muted">
            {done} point pairs{done < 6 ? `; at least 6 are needed` : ""}. Pair points on more than
            one surface, spread over the photo and at different depths.
          </p>
          <label>
            Lens model
            <select
              value={params.model}
              onChange={(e) => setParams({ ...params, model: e.target.value as LensModel })}
            >
              {LENSES.map(([k, t]) => (
                <option key={k} value={k}>
                  {t}
                </option>
              ))}
            </select>
          </label>
          <label>
            Pick uncertainty (px, 1σ)
            <input
              type="number"
              step={0.1}
              value={params.pick_sigma_px}
              onChange={(e) =>
                e.target.valueAsNumber > 0 &&
                setParams({ ...params, pick_sigma_px: e.target.valueAsNumber })
              }
            />
          </label>
          <label>
            Floor elevation (m)
            <input
              type="number"
              step={0.01}
              value={Number(params.floor_z.toFixed(4))}
              onChange={(e) =>
                Number.isFinite(e.target.valueAsNumber) &&
                setParams({ ...params, floor_z: e.target.valueAsNumber })
              }
            />
          </label>
          <button
            onClick={() =>
              requestPick("Click the floor where the subjects stand.", async (hit) => {
                try {
                  const eye = engine()?.cameraProject().eye ?? ([0, 0, 10] as V3);
                  const s = await api.surfaceAt(hit, 0.1, eye);
                  setParams((p) => ({ ...p, floor_z: s.point[2] }));
                } catch (err) {
                  onNotice(String(err));
                }
              })
            }
          >
            Pick the floor
          </button>

          <h3>Subjects</h3>
          {subjects.map((s, i) => {
            const h = heightOf(i);
            return (
              <fieldset key={i} className={i === active ? "active" : ""}>
                <label>
                  Label
                  <input
                    value={s.label}
                    onChange={(e) => patchSubject(i, { label: e.target.value })}
                  />
                </label>
                <label>
                  Frame
                  <select
                    value={s.frame?.evidence_id ?? ""}
                    onChange={(e) =>
                      patchSubject(i, {
                        frame: photos.find((p) => p.evidence_id === Number(e.target.value)) ?? null,
                        feet: null,
                        head: null,
                        matched: null,
                      })
                    }
                  >
                    <option value="">The camera&apos;s photo</option>
                    {photos
                      .filter((p) => p.evidence_id !== photo?.evidence_id)
                      .map((p) => (
                        <option key={p.evidence_id} value={p.evidence_id}>
                          {p.name}
                        </option>
                      ))}
                  </select>
                </label>
                <div className="buttons">
                  <button
                    className={i === active && mode === "feet" ? "primary" : ""}
                    onClick={() => (setActive(i), setMode("feet"), setEditing(true))}
                  >
                    Mark feet{s.feet ? " ✓" : ""}
                  </button>
                  <button
                    className={i === active && mode === "head" ? "primary" : ""}
                    onClick={() => (setActive(i), setMode("head"), setEditing(true))}
                  >
                    Mark head{s.head ? (s.matched ? " (model)" : " ✓") : ""}
                  </button>
                  <button
                    onClick={() => {
                      setSubjects((ss) => [
                        ...ss,
                        { ...s, feet: null, head: null, matched: null, frame: null },
                      ]);
                      setActive(subjects.length);
                    }}
                  >
                    Measure in another frame
                  </button>
                  <button onClick={() => setSubjects((ss) => ss.filter((_, j) => j !== i))}>
                    Remove
                  </button>
                </div>
                {h && (
                  <div>
                    Height <strong>{fmt(h.height.value, h.height.sigma)} m</strong>{" "}
                    <span className="muted">
                      (95 % {h.interval95[0].toFixed(3)}–{h.interval95[1].toFixed(3)} m; head ray{" "}
                      {(h.miss * 1000).toFixed(0)} mm off the vertical)
                    </span>
                  </div>
                )}
                <label>
                  Person model (m)
                  <input
                    className="narrow"
                    inputMode="decimal"
                    placeholder={h ? h.height.value.toFixed(3) : ""}
                    value={s.model}
                    onChange={(e) => patchSubject(i, { model: e.target.value })}
                  />
                </label>
                <label>
                  Pose
                  <select
                    value={s.pose}
                    onChange={(e) => patchSubject(i, { pose: e.target.value as PoseName })}
                  >
                    {Object.keys(POSES).map((p) => (
                      <option key={p}>{p}</option>
                    ))}
                  </select>
                </label>
                <label>
                  Facing (° from +y)
                  <input
                    type="number"
                    step={5}
                    value={s.heading}
                    onChange={(e) => patchSubject(i, { heading: e.target.valueAsNumber || 0 })}
                  />
                </label>
                <button disabled={!h || !(Number(s.model) > 0)} onClick={() => matchModel(i)}>
                  Use the model&apos;s head point
                </button>
              </fieldset>
            );
          })}
          <button
            onClick={() => {
              setSubjects((ss) => [
                ...ss,
                {
                  label: `Subject ${String.fromCharCode(65 + ss.length)}`,
                  feet: null,
                  head: null,
                  matched: null,
                  frame: null,
                  model: "",
                  pose: "standing",
                  heading: 0,
                },
              ]);
              setActive(subjects.length);
            }}
          >
            Add subject
          </button>

          {error && <p className="error">{error}</p>}
          {preview && (
            <div className="trajectory-result">
              <div>
                Camera at{" "}
                <strong>
                  ({preview.solve.camera.position.map((v) => v.toFixed(3)).join(", ")}) m
                </strong>{" "}
                <span className="muted">
                  (1σ {preview.solve.position_sigma.map((v) => (v * 1000).toFixed(0)).join(", ")}{" "}
                  mm)
                </span>
              </div>
              <div className="muted">
                Focal length {fmt(preview.solve.camera.f, preview.solve.f_sigma, 0)} px;{" "}
                {preview.pairs.length} pairs, {preview.solve.rms_px.toFixed(2)} px RMS; χ²{" "}
                {preview.solve.chi2.toFixed(1)} on {preview.solve.dof}
              </div>
              {preview.solve.selection && (
                <div className="muted">Lens: {preview.solve.selection.reason}.</div>
              )}
              {preview.across_frames.map((a) => (
                <div key={a.label}>
                  {a.label}:{" "}
                  <strong>
                    {a.min.toFixed(3)}–{a.max.toFixed(3)} m
                  </strong>{" "}
                  <span className="muted">
                    over {a.frames} frames (mean {a.mean.toFixed(3)} m)
                  </span>
                </div>
              ))}
              {preview.solve.warnings.map((w, i) => (
                <p key={i} className="error">
                  {w}
                </p>
              ))}
            </div>
          )}
          <div className="buttons">
            <button
              onClick={() => {
                setOpen(false);
                setEditing(false);
                setLook(false);
              }}
            >
              Cancel
            </button>
            <button className="primary" disabled={!preview} onClick={() => void saveRun()}>
              Save analysis
            </button>
          </div>
        </div>
      )}
      {drawn && (
        <div className="buttons">
          <label>
            <input type="checkbox" checked={look} onChange={(e) => setLook(e.target.checked)} />{" "}
            Look through this camera
          </label>
          {look && (
            <label>
              Photo over the scene
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={opacity}
                onChange={(e) => setOpacity(e.target.valueAsNumber)}
              />
            </label>
          )}
        </div>
      )}
      {open && editing && photo && (
        <div className="stain-editor">
          <div className="stain-editor-bar">
            <strong>{shownPhoto?.name}</strong>
            {(
              [
                ["pairs", "Point pairs"],
                ["feet", "Feet"],
                ["head", "Head"],
              ] as [Mode, string][]
            ).map(([m, label]) => (
              <button
                key={m}
                className={mode === m ? "primary" : ""}
                disabled={m !== "pairs" && !subjects[active]}
                onClick={() => setMode(m)}
              >
                {label}
                {m !== "pairs" && subjects[active] ? `: ${subjects[active].label}` : ""}
              </button>
            ))}
            {mode === "pairs" && pairs.length > 0 && (
              <button onClick={() => setPairs((ps) => ps.slice(0, -1))}>Remove last pair</button>
            )}
            <button onClick={() => setEditing(false)}>Done</button>
          </div>
          <div className="stain-editor-bar muted">
            {mode === "pairs" &&
              "Click a feature in the photo (a corner, a marker, a fixture), then the same point on the scan. Blue dots: the scan as the solved camera sees it; red: each pair's residual, ten times longer."}
            {mode === "feet" &&
              "Click the floor midway between the subject's feet (for a stride, under the body's centre)."}
            {mode === "head" &&
              "Click the top of the subject's head. Or set the person model's height and use its head point."}
          </div>
          {image ? (
            <PhotoEditor
              image={image}
              marks={{
                pairs: onFrame ? [] : pairs.map((q) => ({ px: q.px, done: q.pick !== null })),
                corners: [],
                edges: [],
                seed: null,
                tail: null,
              }}
              onClick={click}
              draw={draw}
            />
          ) : (
            <p className="muted">Loading the photo…</p>
          )}
        </div>
      )}
      {cameras.map((r) => (
        <div key={r.id} className={`dg-object${shown === r.id ? " active" : ""}`}>
          <button className="link" onClick={() => setShown(shown === r.id ? null : r.id)}>
            {r.name}
            {r.withdrawn ? " (withdrawn)" : ""}
          </button>
          <div className="muted">{r.record.summary}</div>
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

/** Report and withdraw buttons for a stored analysis. */
export function RecordButtons({
  id,
  name,
  withdrawn,
  setRecords,
  onNotice,
}: {
  id: number;
  name: string;
  withdrawn: boolean;
  setRecords: (r: AnalysisRecord[]) => void;
  onNotice: (m: string | null) => void;
}) {
  return (
    <div className="buttons">
      <button
        onClick={async () => {
          const path = await saveDialog({
            defaultPath: `${name}.pdf`,
            filters: [{ name: "PDF", extensions: ["pdf"] }],
          });
          if (!path) return;
          try {
            const sha = await api.analysisReport(id, path);
            onNotice(`Report saved to ${path} (SHA-256 ${sha}; recorded in the audit log).`);
          } catch (e) {
            onNotice(String(e));
          }
        }}
      >
        Report PDF…
      </button>
      {!withdrawn && (
        <button
          onClick={async () => {
            const why = window.prompt(
              "Why is this analysis withdrawn? (recorded in the audit log)",
            );
            if (!why?.trim()) return;
            try {
              setRecords(await api.analysisWithdraw(id, why));
            } catch (e) {
              onNotice(String(e));
            }
          }}
        >
          Withdraw…
        </button>
      )}
    </div>
  );
}
