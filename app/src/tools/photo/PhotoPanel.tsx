// Photogrammetry, in the 3D view's side panel: COLMAP's setup (installed by the examiner; Locus
// runs it and records its version and hash), a reconstruction from the project's photos or a
// video, then scaling (known distances or control points clicked in the photos, or the photos'
// GPS) and import of the scaled point cloud as evidence, with the run stored as an analysis.
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import {
  api,
  type AnalysisRecord,
  type PhotoClick,
  type PhotoJob,
  type PhotoScale,
  type PhotoScaleRecord,
  type PhotoSetup,
  type PhotoSources,
} from "../../api";
import type { PickHit } from "../../viewer3d/pointcloud";
import { PhotoEditor } from "../bloodstain/PhotoEditor";
import { RecordButtons } from "../camera/CameraPanel";

type P2 = [number, number];

/** A target clicked in photos: a distance's end, or a control point. */
interface Target {
  label: string;
  clicks: PhotoClick[];
}
interface Distance {
  label: string;
  a: Target;
  b: Target;
  length: string;
  sigma: string;
}
interface Gcp {
  target: Target;
  x: string;
  y: string;
  z: string;
  pick: PickHit | null;
  check: boolean;
}

const LICENCES = [
  "COLMAP itself: BSD-3-Clause.",
  "Bundled in COLMAP's source: SiftGPU, licensed for educational, research and non-profit use only (used for GPU feature extraction and matching; the CPU-only setting avoids it); LSD, AGPL-3.0 (line detection, not used by Locus); PoissonRecon, MIT; VLFeat, BSD-2-Clause (the CPU SIFT).",
  "Linked in the official builds: Qt (LGPL-3, the GUI), CGAL (GPL-3/LGPL-3, meshing), Ceres Solver (BSD-3), Eigen (MPL-2.0), Boost (BSL-1.0), SQLite (public domain), OpenImageIO and its codecs (Apache-2.0 and permissive), SuiteSparse (parts LGPL/GPL) and, in the CUDA build, NVIDIA's CUDA runtime (NVIDIA's licence).",
  "Locus doesn't ship COLMAP: your agency installs it and decides whether these terms suit its use. Check the licence files in the COLMAP folder you install; this summary was made for COLMAP 4.2.",
];

function pad(n: number) {
  return n.toString();
}

export function PhotoPanel({
  requestPick,
  onNotice,
}: {
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  onNotice: (m: string | null) => void;
}) {
  const [setup, setSetup] = useState<PhotoSetup | null>(null);
  const [path, setPath] = useState("");
  const [sources, setSources] = useState<PhotoSources | null>(null);
  const [kind, setKind] = useState<"photos" | "video">("photos");
  const [chosen, setChosen] = useState<number[]>([]);
  const [video, setVideo] = useState<number | null>(null);
  const [interval, setInterval_] = useState("0.5");
  const [model, setModel] = useState("OPENCV");
  const [single, setSingle] = useState(false);
  const [dense, setDense] = useState(true);
  const [size, setSize] = useState("4800");
  const [denseSize, setDenseSize] = useState("2000");
  const [running, setRunning] = useState<{
    stage: string;
    done: number;
    total: number;
    line: string;
  } | null>(null);
  const [job, setJob] = useState<PhotoJob | null>(null);
  const [method, setMethod] = useState<"distances" | "gcps" | "gps">("distances");
  const [distances, setDistances] = useState<Distance[]>([]);
  const [gcps, setGcps] = useState<Gcp[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [image, setImage] = useState<string>("");
  const [img, setImg] = useState<HTMLImageElement | null>(null);
  const [checked, setChecked] = useState<PhotoScaleRecord | null>(null);
  const [name, setName] = useState("Photogrammetry 1");
  const [records, setRecords] = useState<AnalysisRecord[]>([]);

  const refresh = () => {
    api.photoSetup().then(
      (s) => {
        setSetup(s);
        setPath(s.configured ?? s.candidates[0] ?? "");
      },
      (e) => onNotice(String(e)),
    );
    api.photoSources().then(
      (s) => {
        setSources(s);
        setChosen(s.images.map((i) => i.evidence_id));
        setVideo(s.videos[0]?.evidence_id ?? null);
      },
      (e) => onNotice(String(e)),
    );
    api.photoJob().then(setJob, () => {});
    api.analyses().then(setRecords, () => {});
  };
  // eslint-disable-next-line react-hooks/exhaustive-deps -- once, on open
  useEffect(refresh, []);

  useEffect(() => {
    const a = listen<{ stage: string; done: number; total: number; line: string }>(
      "photo-progress",
      ({ payload }) => setRunning(payload),
    );
    const b = listen<{ job: PhotoJob | null; error: string | null }>(
      "photo-done",
      ({ payload }) => {
        setRunning(null);
        if (payload.error) onNotice(payload.error);
        if (payload.job) {
          setJob(payload.job);
          setImage(payload.job.registered[0] ?? "");
        }
      },
    );
    return () => {
      void a.then((f) => f());
      void b.then((f) => f());
    };
  }, [onNotice]);

  // The photo shown for clicking targets.
  useEffect(() => {
    if (!image) return;
    let live = true;
    let url = "";
    api.photoImage(image).then(
      (b) => {
        if (!live) return;
        url = URL.createObjectURL(new Blob([b]));
        const i = new Image();
        i.onload = () => live && setImg(i);
        i.src = url;
      },
      (e) => onNotice(String(e)),
    );
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [image, onNotice]);

  const saveSetup = async (p: string | null, cpu: boolean) => {
    try {
      setSetup(await api.photoSetupSet(p, cpu));
      onNotice(null);
    } catch (e) {
      onNotice(String(e));
    }
  };

  const run = async () => {
    try {
      setJob(null);
      setChecked(null);
      setRunning({ stage: "starting", done: 0, total: 0, line: "" });
      await api.photoRun({
        source:
          kind === "photos"
            ? { kind, evidence_ids: chosen }
            : { kind, evidence_id: video ?? -1, interval: Number(interval) },
        camera_model: model,
        single_camera: single,
        dense,
        max_image_size: Number(size) || 0,
        dense_max_image_size: Number(denseSize) || 0,
      });
    } catch (e) {
      setRunning(null);
      onNotice(String(e));
    }
  };

  // Targets by key: "d<i>a", "d<i>b", "g<i>".
  const target = (key: string): Target | null => {
    const i = Number(key.slice(1, key.length - (key[0] === "d" ? 1 : 0)));
    if (key[0] === "d") return distances[i]?.[key.endsWith("a") ? "a" : "b"] ?? null;
    return gcps[i]?.target ?? null;
  };
  const setTarget = (key: string, t: Target) => {
    const i = Number(key.slice(1, key.length - (key[0] === "d" ? 1 : 0)));
    if (key[0] === "d")
      setDistances((ds) =>
        ds.map((d, j) => (j === i ? { ...d, [key.endsWith("a") ? "a" : "b"]: t } : d)),
      );
    else setGcps((gs) => gs.map((g, j) => (j === i ? { ...g, target: t } : g)));
  };
  const click = (px: P2) => {
    if (!active || !image) return;
    const t = target(active);
    if (!t) return;
    setTarget(active, {
      ...t,
      clicks: [...t.clicks.filter((c) => c.image !== image), { image, x: px[0], y: px[1] }],
    });
    setChecked(null);
  };

  const scaleRequest = (): PhotoScale | null => {
    if (method === "gps") return { method };
    if (method === "distances")
      return distances.length
        ? {
            method,
            items: distances.map((d) => ({
              label: d.label,
              a: d.a.clicks,
              b: d.b.clicks,
              length: Number(d.length),
              sigma: Number(d.sigma),
            })),
          }
        : null;
    return gcps.length
      ? {
          method,
          items: gcps.map((g) => ({
            label: g.target.label,
            clicks: g.target.clicks,
            world: g.pick ? null : [Number(g.x), Number(g.y), Number(g.z)],
            pick: g.pick,
            check: g.check,
          })),
        }
      : null;
  };

  const check = async () => {
    const r = scaleRequest();
    if (!r) return;
    try {
      setChecked(await api.photoScale(r));
      onNotice(null);
    } catch (e) {
      setChecked(null);
      onNotice(String(e));
    }
  };
  const importCloud = async () => {
    const r = scaleRequest();
    if (!r) return;
    try {
      onNotice("Writing and importing the point cloud…");
      const out = await api.photoImport(name, r);
      setRecords(await api.analyses());
      onNotice(
        `Imported as evidence ${out.evidence_id}; saved as analysis ${out.record.id} (recorded in the audit log).`,
      );
    } catch (e) {
      onNotice(String(e));
    }
  };

  const marks = { pairs: [], corners: [], edges: [], seed: null, tail: null };
  const allTargets: [string, Target][] = [
    ...distances.flatMap((d, i): [string, Target][] => [
      [`d${i}a`, d.a],
      [`d${i}b`, d.b],
    ]),
    ...gcps.map((g, i): [string, Target] => [`g${i}`, g.target]),
  ];
  const photoRuns = records.filter((r) => r.tool === "photogrammetry");
  const f = setup?.found;

  return (
    <section className="panel-section photo">
      <h3>Photogrammetry</h3>
      <details open={!f}>
        <summary>
          COLMAP:{" "}
          {f
            ? `${f.banner}${f.supported ? "" : " (too old)"}`
            : setup?.error
              ? "not working"
              : "not set up"}
        </summary>
        <p className="muted">
          Locus runs COLMAP, installed separately. Download it from{" "}
          <span className="selectable">{setup?.releases}</span> (the CUDA build for dense point
          clouds on NVIDIA graphics), unzip it, and choose its COLMAP.bat or bin\colmap.exe. Every
          run records COLMAP&apos;s version and its executable&apos;s SHA-256.
        </p>
        {setup?.error && <p className="error">{setup.error}</p>}
        <label>
          COLMAP
          <input
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="…\COLMAP.bat"
          />
        </label>
        <div className="buttons">
          <button
            onClick={async () => {
              const p = await open({ filters: [{ name: "COLMAP", extensions: ["exe", "bat"] }] });
              if (typeof p === "string") setPath(p);
            }}
          >
            Browse…
          </button>
          <button onClick={() => void saveSetup(path, setup?.cpu_only ?? false)}>Use this</button>
        </div>
        {setup?.candidates
          .filter((c) => c !== f?.path)
          .map((c) => (
            <div key={c} className="muted">
              Found: {c}{" "}
              <button className="link" onClick={() => void saveSetup(c, setup.cpu_only)}>
                use
              </button>
            </div>
          ))}
        {f && (
          <p className="muted">
            {f.cuda
              ? "With CUDA: dense point clouds available."
              : "Without CUDA: sparse reconstructions only."}{" "}
            SHA-256 {f.sha256.slice(0, 16)}…
          </p>
        )}
        <label className="check">
          <input
            type="checkbox"
            checked={setup?.cpu_only ?? false}
            onChange={(e) => void saveSetup(setup?.configured ?? null, e.target.checked)}
          />
          CPU-only feature extraction and matching (COLMAP&apos;s own SIFT, BSD; avoids SiftGPU,
          whose licence is non-commercial; slower)
        </label>
        <details>
          <summary>COLMAP&apos;s component licences</summary>
          <ul>
            {LICENCES.map((l) => (
              <li key={l}>{l}</li>
            ))}
          </ul>
        </details>
      </details>

      {f?.supported && (
        <div className="dg-built">
          <strong>Reconstruct</strong>
          <label>
            From
            <select value={kind} onChange={(e) => setKind(e.target.value as "photos" | "video")}>
              <option value="photos">photos in the project</option>
              <option value="video">a video in the project</option>
            </select>
          </label>
          {kind === "photos" && (
            <div className="photo-list">
              {sources?.images.length === 0 && (
                <p className="muted">Import the photos as evidence first.</p>
              )}
              {sources?.images.map((i) => (
                <label key={i.evidence_id} className="check">
                  <input
                    type="checkbox"
                    checked={chosen.includes(i.evidence_id)}
                    onChange={(e) =>
                      setChosen((c) =>
                        e.target.checked
                          ? [...c, i.evidence_id]
                          : c.filter((x) => x !== i.evidence_id),
                      )
                    }
                  />
                  {i.name} ({i.width}×{i.height})
                </label>
              ))}
            </div>
          )}
          {kind === "video" && (
            <>
              <label>
                Video
                <select value={video ?? ""} onChange={(e) => setVideo(Number(e.target.value))}>
                  {sources?.videos.length === 0 && (
                    <option value="">Import a video as evidence first</option>
                  )}
                  {sources?.videos.map((v) => (
                    <option key={v.evidence_id} value={v.evidence_id}>
                      {v.name}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                One frame every (s)
                <input
                  className="narrow"
                  inputMode="decimal"
                  value={interval}
                  onChange={(e) => setInterval_(e.target.value)}
                />
              </label>
            </>
          )}
          <label>
            Camera model
            <select value={model} onChange={(e) => setModel(e.target.value)}>
              <option value="SIMPLE_RADIAL">SIMPLE_RADIAL (phone, little distortion)</option>
              <option value="OPENCV">OPENCV (most cameras)</option>
              <option value="OPENCV_FISHEYE">OPENCV_FISHEYE (fisheye, action cameras)</option>
            </select>
          </label>
          <label className="check">
            <input type="checkbox" checked={single} onChange={(e) => setSingle(e.target.checked)} />
            One camera for every photo (same body and lens, fixed zoom)
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={dense && f.cuda}
              disabled={!f.cuda}
              onChange={(e) => setDense(e.target.checked)}
            />
            Dense point cloud{f.cuda ? "" : " (needs COLMAP's CUDA build)"}
          </label>
          <label>
            Longest image side for features (px)
            <input
              className="narrow"
              inputMode="numeric"
              value={size}
              onChange={(e) => setSize(e.target.value)}
            />
          </label>
          {dense && f.cuda && (
            <label>
              Longest image side for the dense cloud (px)
              <input
                className="narrow"
                inputMode="numeric"
                value={denseSize}
                onChange={(e) => setDenseSize(e.target.value)}
              />
            </label>
          )}
          <div className="buttons">
            {!running && (
              <button
                className="primary"
                disabled={kind === "photos" ? chosen.length < 3 : video === null}
                onClick={() => void run()}
              >
                Reconstruct
              </button>
            )}
            {running && <button onClick={() => void api.photoCancel()}>Cancel</button>}
          </div>
          {running && (
            <p className="muted">
              {running.stage}
              {running.total > 0
                ? ` ${pad(running.done)} of ${running.total}`
                : running.done > 0
                  ? ` (${running.done} images placed)`
                  : ""}
              …
            </p>
          )}
        </div>
      )}

      {job && (
        <div className="dg-built">
          <strong>Scale and import</strong>
          <p className="muted">
            {job.registered.length} of {job.images_total} images placed, {job.sparse_points} points,
            reprojection error {job.mean_error_px.toFixed(2)} px
            {job.dense ? "; dense cloud made" : ""} ({job.seconds.toFixed(0)} s).
            {job.other_models.length > 0 &&
              ` Separate groups not joined: ${job.other_models.join(", ")} images.`}
          </p>
          {job.note && <p className="error">{job.note}</p>}
          <label>
            Scale by
            <select
              value={method}
              onChange={(e) => {
                setMethod(e.target.value as typeof method);
                setChecked(null);
              }}
            >
              <option value="distances">known distances, clicked in the photos</option>
              <option value="gcps">control points, clicked in the photos</option>
              <option value="gps" disabled={job.gps < 3}>
                the photos&apos; GPS ({job.gps} with GPS, {job.rtk} RTK)
              </option>
            </select>
          </label>
          {method === "distances" && (
            <>
              {distances.map((d, i) => (
                <fieldset key={i}>
                  <input
                    value={d.label}
                    onChange={(e) =>
                      setDistances((ds) =>
                        ds.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)),
                      )
                    }
                  />
                  <label className="val-field">
                    Length
                    <span>
                      <input
                        className="narrow"
                        inputMode="decimal"
                        value={d.length}
                        onChange={(e) =>
                          setDistances((ds) =>
                            ds.map((x, j) => (j === i ? { ...x, length: e.target.value } : x)),
                          )
                        }
                      />
                      ±
                      <input
                        className="narrow"
                        inputMode="decimal"
                        value={d.sigma}
                        onChange={(e) =>
                          setDistances((ds) =>
                            ds.map((x, j) => (j === i ? { ...x, sigma: e.target.value } : x)),
                          )
                        }
                      />
                      <span className="muted"> m (1σ)</span>
                    </span>
                  </label>
                </fieldset>
              ))}
              <div className="buttons">
                <button
                  onClick={() =>
                    setDistances((ds) => [
                      ...ds,
                      {
                        label: `Distance ${ds.length + 1}`,
                        a: { label: "first end", clicks: [] },
                        b: { label: "second end", clicks: [] },
                        length: "",
                        sigma: "0.002",
                      },
                    ])
                  }
                >
                  Add a known distance
                </button>
              </div>
            </>
          )}
          {method === "gcps" && (
            <>
              {gcps.map((g, i) => {
                const set = (p: Partial<Gcp>) =>
                  setGcps((gs) => gs.map((x, j) => (j === i ? { ...x, ...p } : x)));
                return (
                  <fieldset key={i}>
                    <input
                      value={g.target.label}
                      onChange={(e) => set({ target: { ...g.target, label: e.target.value } })}
                    />
                    {!g.pick && (
                      <span>
                        <input
                          className="narrow"
                          placeholder="x"
                          value={g.x}
                          onChange={(e) => set({ x: e.target.value })}
                        />
                        <input
                          className="narrow"
                          placeholder="y"
                          value={g.y}
                          onChange={(e) => set({ y: e.target.value })}
                        />
                        <input
                          className="narrow"
                          placeholder="z"
                          value={g.z}
                          onChange={(e) => set({ z: e.target.value })}
                        />
                        <span className="muted"> m</span>
                      </span>
                    )}
                    <div className="buttons">
                      <button
                        onClick={() =>
                          requestPick(`Click ${g.target.label} on a scan.`, (h) => set({ pick: h }))
                        }
                      >
                        {g.pick ? "Picked on a scan (pick again)" : "Pick on a scan"}
                      </button>
                      {g.pick && (
                        <button onClick={() => set({ pick: null })}>Type coordinates</button>
                      )}
                    </div>
                    <label className="check">
                      <input
                        type="checkbox"
                        checked={g.check}
                        onChange={(e) => set({ check: e.target.checked })}
                      />
                      Check point (held out of the fit)
                    </label>
                  </fieldset>
                );
              })}
              <div className="buttons">
                <button
                  onClick={() =>
                    setGcps((gs) => [
                      ...gs,
                      {
                        target: { label: `GCP ${gs.length + 1}`, clicks: [] },
                        x: "",
                        y: "",
                        z: "",
                        pick: null,
                        check: false,
                      },
                    ])
                  }
                >
                  Add a control point
                </button>
              </div>
            </>
          )}
          {method !== "gps" && allTargets.length > 0 && (
            <>
              <p className="muted">
                Choose a target, then click it in at least two photos (wheel to zoom, drag to pan).
                Photos far apart place it best.
              </p>
              <label>
                Target
                <select value={active ?? ""} onChange={(e) => setActive(e.target.value || null)}>
                  <option value="">Choose…</option>
                  {allTargets
                    .filter(([k]) => (method === "distances" ? k[0] === "d" : k[0] === "g"))
                    .map(([k, t]) => (
                      <option key={k} value={k}>
                        {k[0] === "d"
                          ? `${distances[Number(k.slice(1, -1))]?.label}, ${t.label}`
                          : t.label}{" "}
                        ({t.clicks.length} photos)
                      </option>
                    ))}
                </select>
              </label>
              <label>
                Photo
                <select value={image} onChange={(e) => setImage(e.target.value)}>
                  {job.registered.map((n) => (
                    <option key={n} value={n}>
                      {n}
                    </option>
                  ))}
                </select>
              </label>
              {img && (
                <div className="photo-editor">
                  <PhotoEditor
                    image={img}
                    marks={marks}
                    onClick={click}
                    draw={(ctx, at) => {
                      for (const [k, t] of allTargets)
                        for (const c of t.clicks.filter((c) => c.image === image)) {
                          const [x, y] = at([c.x, c.y]);
                          ctx.strokeStyle = k === active ? "#ff9f0a" : "#3fa9ff";
                          ctx.lineWidth = 2;
                          ctx.beginPath();
                          ctx.arc(x, y, 7, 0, Math.PI * 2);
                          ctx.moveTo(x - 11, y);
                          ctx.lineTo(x + 11, y);
                          ctx.moveTo(x, y - 11);
                          ctx.lineTo(x, y + 11);
                          ctx.stroke();
                        }
                    }}
                  />
                </div>
              )}
            </>
          )}
          <div className="buttons">
            <button onClick={() => void check()}>Check the scaling</button>
          </div>
          {checked && (
            <div className="trajectory-result">
              <div>
                Scale {checked.transform.scale.toFixed(5)} (±
                {(checked.scale_sigma_rel * 100).toFixed(2)} %); RMS{" "}
                {(checked.rms * 1000).toFixed(1)} mm
              </div>
              {checked.rows.map((r) => (
                <div key={r.label} className="muted">
                  {r.label}
                  {r.check ? " (check)" : ""}: {r.target}; residual {(r.residual * 1000).toFixed(1)}{" "}
                  mm
                  {r.angle_deg > 0 ? `, rays ${r.angle_deg.toFixed(0)}° apart` : ""}
                </div>
              ))}
              {checked.warnings.map((w) => (
                <p key={w} className="error">
                  {w}
                </p>
              ))}
            </div>
          )}
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <div className="buttons">
            <button className="primary" disabled={!checked} onClick={() => void importCloud()}>
              Import the point cloud
            </button>
          </div>
        </div>
      )}
      {photoRuns.map((r) => (
        <div key={r.id} className="dg-object">
          <div>
            {r.name}
            {r.withdrawn ? " (withdrawn)" : ""}
          </div>
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
