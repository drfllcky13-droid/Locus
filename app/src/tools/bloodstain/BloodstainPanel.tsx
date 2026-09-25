// Bloodstain area of origin, in the 3D view's side panel. For each stain: choose its photo,
// place the photo on the scan with point pairs (a pixel, then the same point on the cloud),
// find its edge (automatically from a seed and threshold, or by clicking), and mark its
// tail. The backend fits each ellipse and the origin from the stored data; the preview is
// drawn over the scene, and a run is saved as an audit-logged analysis with a PDF report.
import { useUnsaved } from "../../unsaved";
import { HelpButton } from "../../help/Help";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  type Alignment,
  type AnalysisRecord,
  type BloodstainParameters,
  type BloodstainRecord,
  type BloodstainRun,
  type Measured,
  type ScaleCorners,
  type StainInput,
  type StainRequest,
  type StainResult,
} from "../../api";
import { useUnderlayUrl } from "../../diagram2d/UnderlayPanel";
import type { Engine } from "../../viewer3d/engine";
import type { PickHit } from "../../viewer3d/pointcloud";
import { bloodstainOverlay } from "./overlay";
import { PhotoEditor } from "./PhotoEditor";

type P2 = [number, number];
type Photo = Awaited<ReturnType<typeof api.underlayImages>>[number];
type Mode = "pairs" | "scale" | "auto" | "click" | "tail";

interface Draft {
  label: string;
  surface: string;
  photo: Photo;
  pairs: { px: P2; pick: PickHit | null }[];
  /** A scale's four corners and its size (mm, as typed), for a photo not taken square on. */
  scale: { corners: P2[]; width: string; height: string };
  edges: P2[];
  auto: { seed: P2; threshold: number } | null;
  tail: P2 | null;
  excluded: string | null;
  alignment: Alignment | null;
  /** Where the view was when the pairs were picked: the side of the surface the photo is on
   * (the stain's normal points toward it). Fixed then, so moving the view later can't flip it. */
  eye: [number, number, number] | null;
  preview: { input: StainInput; result: StainResult } | null;
  error: string | null;
}

const DEFAULT_PARAMS: BloodstainParameters = {
  floor_z: 0,
  bootstrap: 2000,
  seed: 1,
  include_not_upward: null,
  reference: "project north (+y)",
  reference_deg: 0,
  floor_convergence: true,
};

const fmt = (m: Measured, digits = 1) =>
  `${m.value.toFixed(digits)}° ± ${m.sigma.toFixed(digits)}°`;

/** The perspective correction's corners and size (m), once all four corners and a size are in. */
function scaleCorners(d: Draft): ScaleCorners | null {
  const [w, h] = [Number(d.scale.width), Number(d.scale.height)];
  if (d.scale.corners.length !== 4 || !(w > 0) || !(h > 0)) return null;
  return { corners_px: d.scale.corners as ScaleCorners["corners_px"], size: [w / 1000, h / 1000] };
}

/** How to place the photo, once it has two pairs. */
function alignRequest(d: Draft, eye: [number, number, number]): StainRequest["align"] {
  return {
    pairs: d.pairs.filter((p) => p.pick).map((p) => ({ px: p.px, pick: p.pick! })),
    eye,
    scale: scaleCorners(d),
  };
}

/** The stain as the backend needs it, once it has pairs, edges and a tail. */
function request(d: Draft): StainRequest | null {
  const pairs = d.pairs.filter((p) => p.pick);
  const eye = d.eye;
  if (pairs.length < 2 || d.edges.length < 8 || !d.tail || !eye) return null;
  return {
    label: d.label,
    surface: d.surface,
    photo: d.photo.evidence_id,
    align: alignRequest(d, eye),
    edges: d.edges,
    auto_edge: d.auto,
    tail_px: d.tail,
    excluded: d.excluded,
  };
}

/** Decoded photos, by file and hash. */
const images = new Map<string, Promise<HTMLImageElement>>();
function loadImage(url: string): Promise<HTMLImageElement> {
  if (!images.has(url))
    images.set(
      url,
      new Promise((ok, fail) => {
        const im = new Image();
        im.onload = () => ok(im);
        im.onerror = () => fail(new Error("The photo could not be decoded."));
        im.src = url;
      }),
    );
  return images.get(url)!;
}

/** Automatic edge: a greyscale crop around the seed (at most 800 px a side, downsampled from
 * up to 2400 px), sent to the backend's edge finder, mapped back to photo pixels. */
async function autoEdges(im: HTMLImageElement, seed: P2, threshold: number | null) {
  const [W, H] = [im.naturalWidth, im.naturalHeight];
  const span = 2400;
  const w = Math.min(span, W);
  const h = Math.min(span, H);
  const x0 = Math.min(Math.max(0, Math.round(seed[0] - w / 2)), W - w);
  const y0 = Math.min(Math.max(0, Math.round(seed[1] - h / 2)), H - h);
  const k = Math.max(1, Math.ceil(Math.max(w, h) / 800));
  const [cw, ch] = [Math.ceil(w / k), Math.ceil(h / k)];
  const c = document.createElement("canvas");
  c.width = cw;
  c.height = ch;
  const ctx = c.getContext("2d", { willReadFrequently: true })!;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(im, x0, y0, w, h, 0, 0, cw, ch);
  const rgba = ctx.getImageData(0, 0, cw, ch).data;
  const luma = new Array<number>(cw * ch);
  for (let i = 0; i < cw * ch; i++)
    luma[i] = Math.round(0.299 * rgba[4 * i] + 0.587 * rgba[4 * i + 1] + 0.114 * rgba[4 * i + 2]);
  const [sx, sy] = [w / cw, h / ch];
  const s: P2 = [(seed[0] - x0) / sx, (seed[1] - y0) / sy];
  // Default threshold: halfway between the seed and the crop's border (the background).
  let t = threshold;
  if (t === null) {
    const border: number[] = [];
    for (let x = 0; x < cw; x++) border.push(luma[x], luma[(ch - 1) * cw + x]);
    for (let y = 0; y < ch; y++) border.push(luma[y * cw], luma[y * cw + cw - 1]);
    border.sort((a, b) => a - b);
    const inside = luma[Math.floor(s[1]) * cw + Math.floor(s[0])];
    t = Math.round((inside + border[border.length >> 1]) / 2);
  }
  const edges = await api.bloodstainEdges(luma, cw, ch, s, t);
  return {
    threshold: t,
    edges: edges.map(([x, y]) => [x0 + x * sx, y0 + y * sy] as P2),
  };
}

function StainEditor({
  draft,
  onChange,
  onClose,
  requestPick,
  opacity,
  setOpacity,
}: {
  draft: Draft;
  onChange: (patch: Partial<Draft>) => void;
  onClose: () => void;
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  opacity: number;
  setOpacity: (v: number) => void;
}) {
  const url = useUnderlayUrl(draft.photo.file, draft.photo.sha256);
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  const [mode, setMode] = useState<Mode>(draft.pairs.length < 2 ? "pairs" : "auto");
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    if (url) loadImage(url).then(setImage, (e) => onChange({ error: String(e) }));
  }, [url, onChange]);

  const runAuto = async (seed: P2, threshold: number | null) => {
    if (!image) return;
    setBusy("Finding the edge…");
    try {
      const r = await autoEdges(image, seed, threshold);
      onChange({ edges: r.edges, auto: { seed, threshold: r.threshold }, error: null });
    } catch (e) {
      onChange({ error: String(e) });
    } finally {
      setBusy(null);
    }
  };

  const click = (px: P2) => {
    if (!image) return;
    if (px[0] < 0 || px[1] < 0 || px[0] > image.naturalWidth || px[1] > image.naturalHeight) return;
    if (mode === "scale") {
      const corners = draft.scale.corners.length >= 4 ? [px] : [...draft.scale.corners, px];
      onChange({ scale: { ...draft.scale, corners } });
    } else if (mode === "pairs") {
      const i = draft.pairs.length;
      onChange({ pairs: [...draft.pairs, { px, pick: null }] });
      requestPick(`Click point ${i + 1} on the scan: the same place as in the photo.`, (hit) =>
        onChange({ pairs: [...draft.pairs, { px, pick: hit }] }),
      );
    } else if (mode === "auto") void runAuto(px, null);
    else if (mode === "click") onChange({ edges: [...draft.edges, px], auto: null });
    else onChange({ tail: px });
  };

  const p = draft.preview;
  return (
    <div className="stain-editor">
      <div className="stain-editor-bar">
        <strong>
          Stain {draft.label}: {draft.photo.name}
        </strong>
        {(
          [
            ["pairs", "Point pairs"],
            ["scale", "Scale corners"],
            ["auto", "Edge: automatic"],
            ["click", "Edge: click"],
            ["tail", "Tail"],
          ] as [Mode, string][]
        ).map(([m, label]) => (
          <button key={m} className={mode === m ? "primary" : ""} onClick={() => setMode(m)}>
            {label}
          </button>
        ))}
        <button onClick={onClose}>Done</button>
      </div>
      <div className="stain-editor-bar muted">
        {mode === "pairs" &&
          "Click a feature in the photo (a scale mark, a corner), then the same point on the scan. Two pairs place the photo; three or more check it."}
        {mode === "scale" &&
          "Only for a photo not taken square on: click four corners of a rectangle on the scale, in order around it (C1 to C2 is its width), and give its size. The photo is then corrected for perspective."}
        {mode === "auto" &&
          "Click inside the stain. Adjust the threshold until the red edge follows it."}
        {mode === "click" &&
          "Click points around the stain's edge (at least 8), not along the tail."}
        {mode === "tail" && "Click the tip of the tail: the way the droplet was travelling."}
      </div>
      <div className="stain-editor-bar">
        {mode === "auto" && draft.auto && (
          <label>
            Threshold {draft.auto.threshold}
            <input
              type="range"
              min={1}
              max={254}
              value={draft.auto.threshold}
              onChange={(e) => void runAuto(draft.auto!.seed, e.target.valueAsNumber)}
            />
          </label>
        )}
        {mode === "click" && draft.edges.length > 0 && (
          <button onClick={() => onChange({ edges: draft.edges.slice(0, -1), auto: null })}>
            Undo point
          </button>
        )}
        {mode === "scale" && (
          <>
            <label>
              Width
              <input
                className="narrow"
                inputMode="decimal"
                value={draft.scale.width}
                onChange={(e) => onChange({ scale: { ...draft.scale, width: e.target.value } })}
              />{" "}
              mm
            </label>
            <label>
              Height
              <input
                className="narrow"
                inputMode="decimal"
                value={draft.scale.height}
                onChange={(e) => onChange({ scale: { ...draft.scale, height: e.target.value } })}
              />{" "}
              mm
            </label>
            {draft.scale.corners.length > 0 && (
              <button onClick={() => onChange({ scale: { ...draft.scale, corners: [] } })}>
                Clear corners
              </button>
            )}
          </>
        )}
        {mode === "pairs" && draft.pairs.length > 0 && (
          <button onClick={() => onChange({ pairs: draft.pairs.slice(0, -1), alignment: null })}>
            Remove last pair
          </button>
        )}
        <label>
          Photo on the scan
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={opacity}
            onChange={(e) => setOpacity(e.target.valueAsNumber)}
          />
        </label>
        {busy && <span className="muted">{busy}</span>}
      </div>
      {image ? (
        <PhotoEditor
          image={image}
          marks={{
            pairs: draft.pairs.map((q) => ({ px: q.px, done: q.pick !== null })),
            corners: draft.scale.corners,
            edges: draft.edges,
            seed: draft.auto?.seed ?? null,
            tail: draft.tail,
          }}
          onClick={click}
        />
      ) : (
        <p className="muted">Loading the photo…</p>
      )}
      <div className="stain-editor-bar">
        {draft.alignment && (
          <span className="muted">
            {(draft.alignment.pixels_per_metre / 1000).toFixed(1)} px/mm;{" "}
            {draft.alignment.rms === null
              ? "two pairs (exact, no check)"
              : `pairs fit to ${(draft.alignment.rms * 1000).toFixed(1)} mm RMS`}
            {draft.alignment.rectification &&
              `; perspective corrected (stretch ${(draft.alignment.rectification.stretch * 100).toFixed(0)} %, scale over scan ${(draft.alignment.scale_ratio ?? 1).toFixed(3)})`}
          </span>
        )}
        {draft.alignment?.rectification && draft.alignment.rectification.stretch > 0.1 && (
          <span className="error">
            Large perspective correction: the photo is about{" "}
            {((Math.acos(1 / (1 + draft.alignment.rectification.stretch)) * 180) / Math.PI).toFixed(
              0,
            )}
            ° off square-on. Retake it square on if you can.
          </span>
        )}
        {draft.alignment?.scale_ratio != null &&
          // Beyond 2 % and 2.5σ of the pairs' own scale error (their rotation's σ, in radians).
          Math.abs(draft.alignment.scale_ratio - 1) >
            Math.max(0.02, (2.5 * draft.alignment.rotation_sigma_deg * Math.PI) / 180) && (
            <span className="error">
              The scale&apos;s size and the scan disagree by{" "}
              {((draft.alignment.scale_ratio - 1) * 100).toFixed(1)} %: check the size, corners and
              pairs.
            </span>
          )}
        {p && (
          <span>
            {(p.input.width.value * 1000).toFixed(2)} × {(p.input.length.value * 1000).toFixed(2)}{" "}
            mm, impact <strong>{fmt(p.result.impact)}</strong>, direction{" "}
            <strong>{fmt(p.result.directionality)}</strong>
            {p.result.clearly_upward
              ? " (moving upward)"
              : p.result.upward
                ? " (not clearly upward)"
                : " (moving downward)"}
          </span>
        )}
        {draft.error && <span className="error">{draft.error}</span>}
      </div>
    </div>
  );
}

export function BloodstainPanel({
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
  const [name, setName] = useState("Area of origin 1");
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const [editing, setEditing] = useState<number | null>(null);
  const [params, setParams] = useState(DEFAULT_PARAMS);
  const [includeAll, setIncludeAll] = useState(false);
  const [reason, setReason] = useState("");
  const [result, setResult] = useState<BloodstainRun | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  useUnsaved("Bloodstain area of origin", open && drafts.length > 0);
  const [shown, setShown] = useState<number | null>(null);
  const [opacity, setOpacity] = useState(0.7);
  const [adding, setAdding] = useState<number | "">("");
  const eye = useCallback(
    () => engine()?.cameraProject().eye ?? ([0, 0, 10] as [number, number, number]),
    [engine],
  );

  useEffect(() => {
    api.analyses().then(setRecords, (e) => onNotice(String(e)));
  }, [onNotice]);

  const parameters: BloodstainParameters = {
    ...params,
    include_not_upward: includeAll ? reason : null,
  };

  const patch = useCallback((i: number, p: Partial<Draft>) => {
    setDrafts((ds) => ds.map((d, j) => (j === i ? { ...d, ...p } : d)));
  }, []);

  // Re-place the photo whenever its pairs change, and re-fit the stain whenever anything does.
  const aligned = useRef(new Map<number, string>());
  useEffect(() => {
    drafts.forEach((d, i) => {
      const done = d.pairs.filter((q) => q.pick);
      const key = JSON.stringify([done.map((q) => [q.px, q.pick]), scaleCorners(d)]);
      if (aligned.current.get(i) === key) return;
      aligned.current.set(i, key);
      if (done.length < 2) return patch(i, { alignment: null });
      const e = d.eye ?? eye();
      if (!d.eye) patch(i, { eye: e });
      api.bloodstainAlign(alignRequest(d, e)).then(
        (alignment) => patch(i, { alignment, error: null }),
        (e) => patch(i, { alignment: null, error: String(e) }),
      );
    });
  }, [drafts, eye, patch]);
  const fitted = useRef(new Map<number, string>());
  useEffect(() => {
    drafts.forEach((d, i) => {
      const req = request(d);
      const key = JSON.stringify(req && { ...req, align: req.align.pairs });
      if (fitted.current.get(i) === key) return;
      fitted.current.set(i, key);
      if (!req) return patch(i, { preview: null });
      api.bloodstainStain(req, parameters).then(
        (preview) => patch(i, { preview, error: null }),
        (e) => patch(i, { preview: null, error: String(e) }),
      );
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- parameters only change the angles' reference
  }, [drafts, eye, patch]);

  // The whole run, once enough stains are ready.
  const ready = drafts.map(request).filter((r): r is StainRequest => r !== null);
  const readyKey = JSON.stringify([ready, parameters]);
  useEffect(() => {
    if (!open || ready.length < 4) return;
    let live = true;
    const t = setTimeout(() => {
      api.bloodstainPreview({ stains: ready, parameters }).then(
        (r) => live && (setResult(r), setFailure(null)),
        (e) => live && (setResult(null), setFailure(String(e))),
      );
    }, 300);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- readyKey stands for both
  }, [open, readyKey]);
  const preview = open && ready.length >= 4 ? result : null;
  const error = open && ready.length >= 4 ? failure : null;

  // What's drawn: the preview and the photo being edited, else a shown record.
  const stains = records.filter((r): r is BloodstainRecord => r.tool === "bloodstain");
  const drawn = open ? preview : (stains.find((r) => r.id === shown)?.record ?? null);
  const current = editing !== null ? drafts[editing] : null;
  const photoUrl = useUnderlayUrl(current?.photo.file ?? "", current?.photo.sha256 ?? "");
  useEffect(() => {
    const e = engine();
    if (!e) return;
    const layer =
      current?.alignment && photoUrl
        ? {
            url: photoUrl,
            width: current.photo.width,
            height: current.photo.height,
            alignment: current.alignment,
            opacity,
          }
        : null;
    e.setAnalysisOverlay(
      "bloodstain",
      drawn || layer ? bloodstainOverlay(drawn, layer, e.origin, () => e.requestRender()) : null,
    );
  }, [drawn, current?.alignment, current?.photo, photoUrl, opacity, engine, origin]);
  useEffect(() => () => engine()?.setAnalysisOverlay("bloodstain", null), [engine]);

  const addStain = () => {
    const ph = photos.find((p) => p.evidence_id === adding);
    if (!ph) return;
    const label = String(Math.max(0, ...drafts.map((d) => Number(d.label) || 0)) + 1);
    setDrafts((ds) => [
      ...ds,
      {
        label,
        surface: ds[ds.length - 1]?.surface ?? "wall",
        photo: ph,
        pairs: [],
        scale: { corners: [], width: "", height: "" },
        edges: [],
        auto: null,
        tail: null,
        excluded: null,
        alignment: null,
        eye: null,
        preview: null,
        error: null,
      },
    ]);
    setEditing(drafts.length);
    setAdding("");
  };

  const saveRun = async () => {
    try {
      const rec = await api.bloodstainSave(name, { stains: ready, parameters }, null);
      setRecords(await api.analyses());
      setOpen(false);
      setDrafts([]);
      setEditing(null);
      setShown(rec.id);
      onNotice(`Saved as analysis ${rec.id}: ${rec.record.summary} (recorded in the audit log).`);
    } catch (e) {
      onNotice(String(e));
    }
  };

  const num = (label: string, value: number, set: (v: number) => void, step = 0.01) => (
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

  return (
    <section className="panel-section bloodstain">
      <h3 className="with-help">
        Bloodstain area of origin <HelpButton topic="bloodstain" />
      </h3>
      {!open && (
        <button
          onClick={() => {
            setOpen(true);
            setDrafts([]);
            setName(`Area of origin ${stains.length + 1}`);
            api.underlayImages().then(setPhotos, (e) => onNotice(String(e)));
          }}
        >
          New area of origin…
        </button>
      )}
      {open && (
        <div className="dg-built">
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <p className="muted">
            Straight-line paths ignore gravity and drag, so the origin tends to come out too high.
            Only wall stains clearly moving upward are used unless you include the rest with a
            reason.
          </p>
          {drafts.map((d, i) => (
            <fieldset key={i}>
              <legend>
                Stain {d.label}: {d.photo.name}
              </legend>
              <label>
                Surface
                <input value={d.surface} onChange={(e) => patch(i, { surface: e.target.value })} />
              </label>
              <div className="muted">
                {d.pairs.filter((q) => q.pick).length} pairs, {d.edges.length} edge points,{" "}
                {d.tail ? "tail marked" : "no tail"}
              </div>
              {d.preview && (
                <div className="muted">
                  Impact {fmt(d.preview.result.impact)}, direction{" "}
                  {fmt(d.preview.result.directionality)}
                  {preview?.stains[ready.findIndex((r) => r.label === d.label)] &&
                    (() => {
                      const s = preview.stains[ready.findIndex((r) => r.label === d.label)];
                      return s.used
                        ? `; residual ${(s.residual * 1000).toFixed(0)} mm (${s.residual_sigmas.toFixed(1)}σ)`
                        : `; not used: ${s.not_used}`;
                    })()}
                </div>
              )}
              {d.error && editing !== i && <p className="error">{d.error}</p>}
              <label>
                <input
                  type="checkbox"
                  checked={d.excluded !== null}
                  onChange={(e) => patch(i, { excluded: e.target.checked ? "" : null })}
                />{" "}
                Exclude
              </label>
              {d.excluded !== null && (
                <label>
                  Reason (in the report)
                  <input
                    value={d.excluded}
                    onChange={(e) => patch(i, { excluded: e.target.value })}
                  />
                </label>
              )}
              <div className="buttons">
                <button onClick={() => setEditing(editing === i ? null : i)}>
                  {editing === i ? "Close photo" : "Edit on photo…"}
                </button>
                <button
                  onClick={() => {
                    setDrafts((ds) => ds.filter((_, j) => j !== i));
                    aligned.current.clear();
                    fitted.current.clear();
                    setEditing(null);
                  }}
                >
                  Remove
                </button>
              </div>
            </fieldset>
          ))}
          <label>
            Add a stain from its photo
            <select
              value={adding}
              onChange={(e) => setAdding(e.target.value ? Number(e.target.value) : "")}
            >
              <option value="">Choose an image…</option>
              {photos.map((p) => (
                <option key={p.evidence_id} value={p.evidence_id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          <button disabled={adding === ""} onClick={addStain}>
            Add stain
          </button>
          {photos.length === 0 && (
            <p className="muted">No images in the evidence. Import the stain photos first.</p>
          )}

          <h3>Settings</h3>
          {num("Floor elevation (m)", params.floor_z, (floor_z) =>
            setParams({ ...params, floor_z }),
          )}
          <button
            onClick={() =>
              requestPick("Click the floor.", async (hit) => {
                try {
                  const s = await api.surfaceAt(hit, 0.1, eye());
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
            "Bootstrap resamples",
            params.bootstrap,
            (bootstrap) => bootstrap >= 10 && setParams({ ...params, bootstrap }),
            100,
          )}
          <label>
            <input
              type="checkbox"
              checked={includeAll}
              onChange={(e) => setIncludeAll(e.target.checked)}
            />{" "}
            Include stains not clearly moving upward
          </label>
          {includeAll && (
            <label>
              Reason (in the report)
              <input value={reason} onChange={(e) => setReason(e.target.value)} />
            </label>
          )}
          <label>
            <input
              type="checkbox"
              checked={params.floor_convergence}
              onChange={(e) => setParams({ ...params, floor_convergence: e.target.checked })}
            />{" "}
            Also find where floor stains converge in plan (a separate 2-D result)
          </label>

          {ready.length < 4 && (
            <p className="muted">
              {ready.length} of {drafts.length} stains ready; at least 4 are needed.
            </p>
          )}
          {error && <p className="error">{error}</p>}
          {preview && (
            <div className="trajectory-result">
              <div>
                Origin{" "}
                <strong>({preview.origin.point.map((v) => v.toFixed(3)).join(", ")}) m</strong>
              </div>
              <div>
                Height above the floor{" "}
                <strong>
                  {preview.origin.height.value.toFixed(2)} ±{" "}
                  {preview.origin.height.sigma.toFixed(2)} m
                </strong>
              </div>
              <div className="muted">
                95 % region half-axes{" "}
                {preview.origin.ellipsoid.semi_axes.map((v) => v.toFixed(2)).join(" × ")} m, from{" "}
                {preview.origin.stains_used} of {preview.stains.length} stains; χ²{" "}
                {preview.origin.chi2.toFixed(1)} on {preview.origin.dof}
              </div>
              {preview.conventional && (
                <div className="muted">
                  Conventional point (perpendicular distances, for comparison){" "}
                  {preview.conventional.point.map((v) => v.toFixed(3)).join(", ")} m,{" "}
                  {(preview.conventional.shift * 1000).toFixed(0)} mm from the result
                </div>
              )}
              {preview.origin.near_round_share > 0.5 && (
                <p className="error">
                  Near-round stains (impact 70° or more) carry{" "}
                  {(preview.origin.near_round_share * 100).toFixed(0)} % of the fit: their angles
                  are poorly determined. Check their edges and tails.
                </p>
              )}
              {preview.convergence && (
                <div>
                  Floor stains converge in plan at{" "}
                  <strong>
                    x {preview.convergence.point[0].toFixed(3)}, y{" "}
                    {preview.convergence.point[1].toFixed(3)} m
                  </strong>{" "}
                  <span className="muted">
                    (95 % half-axes{" "}
                    {preview.convergence.semi_axes.map((v) => v.toFixed(2)).join(" × ")} m, from{" "}
                    {preview.convergence.stains.length} floor stains; separate from the origin)
                  </span>
                </div>
              )}
              {preview.convergence_note && <p className="muted">{preview.convergence_note}.</p>}
              {preview.origin.dof > 0 && preview.origin.chi2 / preview.origin.dof > 2 && (
                <p className="error">
                  The stains scatter more than their stated uncertainties: check for stains from
                  another event, or a wrong tail.
                </p>
              )}
            </div>
          )}
          <div className="buttons">
            <button
              onClick={() => {
                setOpen(false);
                setEditing(null);
              }}
            >
              Cancel
            </button>
            <button
              className="primary"
              disabled={!preview || (includeAll && !reason.trim())}
              onClick={() => void saveRun()}
            >
              Save analysis
            </button>
          </div>
        </div>
      )}
      {current && editing !== null && (
        <StainEditor
          key={editing}
          draft={current}
          onChange={(p) => patch(editing, p)}
          onClose={() => setEditing(null)}
          requestPick={requestPick}
          opacity={opacity}
          setOpacity={setOpacity}
        />
      )}
      {stains.map((r) => (
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
                  const why = window.prompt(
                    "Why is this analysis withdrawn? (recorded in the audit log)",
                  );
                  if (!why?.trim()) return;
                  try {
                    setRecords(await api.analysisWithdraw(r.id, why));
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
