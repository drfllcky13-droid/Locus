// Underlays: add an aerial image or a point-cloud slice, calibrate an image from known points,
// set opacity, remove. The calibration's clicks come from the canvas (DiagramEditor).
import { useEffect, useState } from "react";
import { api } from "../api";
import type { Entity, EntityInput, Pt } from "./model";
import { calibrate, type Pair } from "./underlay";

export type UnderlayEntity = Extract<Entity, { kind: "underlay" }>;

/** An image calibration in progress: image points clicked so far and their typed positions. */
export interface Calibrating {
  id: string;
  rows: { pixel: Pt; x: string; y: string }[];
}

const urls = new Map<string, Promise<string>>();

/** Object URL of an underlay's image, fetched once per file and hash. */
export function useUnderlayUrl(file: string, sha256: string): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!file) return;
    const key = `${file}#${sha256}`;
    if (!urls.has(key))
      urls.set(
        key,
        api.underlayBytes(file, sha256).then((b) => URL.createObjectURL(new Blob([b]))),
      );
    let live = true;
    urls.get(key)!.then(
      (u) => live && setUrl(u),
      () => urls.delete(key),
    );
    return () => {
      live = false;
    };
  }, [file, sha256]);
  return url;
}

const fmt = (m: number) => `${(m * 1000).toFixed(1)} mm`;

export function UnderlayPanel({
  underlays,
  points,
  view,
  calibrating,
  setCalibrating,
  add,
  update,
  remove,
  onNotice,
}: {
  underlays: UnderlayEntity[];
  /** Reference and measured points, to use as known positions. */
  points: Extract<Entity, { kind: "point" }>[];
  /** The visible part of the drawing: top-left corner and width (m). */
  view: { topLeft: Pt; width: number };
  calibrating: Calibrating | null;
  setCalibrating: (c: Calibrating | null) => void;
  add: (e: EntityInput) => void;
  update: (e: UnderlayEntity) => void;
  remove: (id: string) => void;
  onNotice: (s: string) => void;
}) {
  const [images, setImages] = useState<Awaited<ReturnType<typeof api.underlayImages>> | null>(null);
  const [slice, setSlice] = useState<{ zMin: number; zMax: number; resolution: number } | null>(
    null,
  );
  const [busy, setBusy] = useState(false);

  const target = calibrating && underlays.find((u) => u.id === calibrating.id);
  const pairs: Pair[] | null = calibrating
    ? calibrating.rows.flatMap((r) => {
        const [x, y] = [parseFloat(r.x), parseFloat(r.y)];
        return Number.isFinite(x) && Number.isFinite(y) ? [{ pixel: r.pixel, world: [x, y] }] : [];
      })
    : null;
  let fit: ReturnType<typeof calibrate> | string | null = null;
  if (pairs && pairs.length >= 2) {
    try {
      fit = calibrate(pairs);
    } catch (e) {
      fit = String(e instanceof Error ? e.message : e);
    }
  }

  const apply = async () => {
    if (!target || !fit || typeof fit === "string" || !pairs) return;
    const calibration = { pairs, residuals: fit.residuals, rms: fit.rms };
    update({ ...target, placement: fit.placement, calibration });
    setCalibrating(null);
    try {
      await api.underlayCalibrated({
        file: target.file,
        sha256: target.sha256,
        ...calibration,
        placement: fit.placement,
      });
      onNotice(
        `Underlay calibrated from ${pairs.length} points: ${(fit.placement.pixel * 1000).toFixed(2)} mm per pixel, RMS residual ${fmt(fit.rms)} (recorded in the audit log).`,
      );
    } catch (e) {
      onNotice(String(e));
    }
  };

  return (
    <div className="dg-underlays">
      <h3>Underlays</h3>
      {underlays.map((u) => (
        <div key={u.id} className="dg-underlay">
          <div className="dg-underlay-name" title={`${u.file}\nSHA-256 ${u.sha256}`}>
            {u.name}
          </div>
          <div className="muted">
            {u.slice
              ? `Slice ${u.slice.zMin}–${u.slice.zMax} m, ${u.slice.resolution * 1000} mm/px`
              : u.calibration
                ? `Calibrated, ${u.calibration.pairs.length} points, RMS ${fmt(u.calibration.rms)}`
                : "Not calibrated"}
          </div>
          <label className="inline">
            Opacity
            <input
              type="range"
              min={0.1}
              max={1}
              step={0.05}
              value={u.opacity}
              onChange={(e) => update({ ...u, opacity: e.target.valueAsNumber })}
            />
          </label>
          <div className="buttons">
            {!u.slice && (
              <button onClick={() => setCalibrating({ id: u.id, rows: [] })}>Calibrate…</button>
            )}
            <button onClick={() => remove(u.id)}>Remove</button>
          </div>
        </div>
      ))}

      {target && calibrating && (
        <div className="dg-calibrate">
          <h3>Calibrate {target.name}</h3>
          <p className="muted">
            Click a feature on the image whose position you know, then give its position (or pick a
            point in the drawing). Two points place the image exactly; add a third or more to check
            the fit.
          </p>
          {calibrating.rows.map((r, i) => {
            const set = (patch: Partial<(typeof calibrating.rows)[number]>) =>
              setCalibrating({
                ...calibrating,
                rows: calibrating.rows.map((x, j) => (j === i ? { ...x, ...patch } : x)),
              });
            const residual =
              fit && typeof fit !== "string" && pairs
                ? fit.residuals[pairs.findIndex((p) => p.pixel === r.pixel)]
                : undefined;
            return (
              <fieldset key={i}>
                <legend>
                  Point {i + 1} (pixel {r.pixel[0].toFixed(0)}, {r.pixel[1].toFixed(0)})
                </legend>
                {points.length > 0 && (
                  <select
                    value=""
                    onChange={(e) => {
                      const p = points.find((q) => q.id === e.target.value);
                      if (p) set({ x: p.at[0].toFixed(3), y: p.at[1].toFixed(3) });
                    }}
                  >
                    <option value="">Use a point…</option>
                    {points.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.label || `(${p.at[0].toFixed(2)}, ${p.at[1].toFixed(2)})`}
                      </option>
                    ))}
                  </select>
                )}
                <label>
                  x (m)
                  <input value={r.x} onChange={(e) => set({ x: e.target.value })} />
                </label>
                <label>
                  y (m)
                  <input value={r.y} onChange={(e) => set({ y: e.target.value })} />
                </label>
                {residual !== undefined && <div className="muted">Residual {fmt(residual)}</div>}
                <button
                  onClick={() =>
                    setCalibrating({
                      ...calibrating,
                      rows: calibrating.rows.filter((_, j) => j !== i),
                    })
                  }
                >
                  Remove
                </button>
              </fieldset>
            );
          })}
          {typeof fit === "string" && <p className="error">{fit}</p>}
          {fit && typeof fit !== "string" && (
            <p className="muted">
              {(fit.placement.pixel * 1000).toFixed(2)} mm per pixel, turned{" "}
              {((fit.placement.rotation * 180) / Math.PI).toFixed(2)}°.{" "}
              {pairs && pairs.length > 2
                ? `RMS residual ${fmt(fit.rms)}.`
                : "Two points fit exactly: there is no check on them."}
            </p>
          )}
          <div className="buttons">
            <button onClick={() => setCalibrating(null)}>Cancel</button>
            <button
              className="primary"
              disabled={!fit || typeof fit === "string"}
              onClick={() => void apply()}
            >
              Apply
            </button>
          </div>
        </div>
      )}

      {images ? (
        <div className="dg-calibrate">
          {images.length === 0 && (
            <p className="muted">No images in the evidence. Import a JPEG or PNG first.</p>
          )}
          {images.map((im) => (
            <button
              key={im.evidence_id}
              onClick={() => {
                // Fit the image across the view until it is calibrated.
                const pixel = view.width / im.width;
                add({
                  kind: "underlay",
                  name: im.name,
                  file: im.file,
                  sha256: im.sha256,
                  width: im.width,
                  height: im.height,
                  placement: { origin: view.topLeft, pixel, rotation: 0 },
                  opacity: 0.6,
                  calibration: null,
                  slice: null,
                });
                setImages(null);
              }}
            >
              {im.name} ({im.width} × {im.height})
            </button>
          ))}
          <button onClick={() => setImages(null)}>Cancel</button>
        </div>
      ) : (
        <button
          onClick={() => api.underlayImages().then(setImages, (e: unknown) => onNotice(String(e)))}
        >
          Add aerial image…
        </button>
      )}

      {slice ? (
        <div className="dg-calibrate">
          <label>
            From height (m)
            <input
              type="number"
              step={0.1}
              value={slice.zMin}
              onChange={(e) => setSlice({ ...slice, zMin: e.target.valueAsNumber })}
            />
          </label>
          <label>
            To height (m)
            <input
              type="number"
              step={0.1}
              value={slice.zMax}
              onChange={(e) => setSlice({ ...slice, zMax: e.target.valueAsNumber })}
            />
          </label>
          <label>
            Resolution (mm per pixel)
            <input
              type="number"
              min={1}
              value={slice.resolution * 1000}
              onChange={(e) => setSlice({ ...slice, resolution: e.target.valueAsNumber / 1000 })}
            />
          </label>
          <div className="buttons">
            <button onClick={() => setSlice(null)}>Cancel</button>
            <button
              className="primary"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                try {
                  const r = await api.underlaySlice(slice.zMin, slice.zMax, slice.resolution);
                  add({
                    kind: "underlay",
                    name: `Slice ${slice.zMin}–${slice.zMax} m`,
                    file: r.file,
                    sha256: r.sha256,
                    width: r.width,
                    height: r.height,
                    placement: { origin: r.origin, pixel: r.resolution, rotation: 0 },
                    opacity: 1,
                    calibration: null,
                    slice: { ...slice, points: r.points },
                  });
                  onNotice(
                    `Slice of ${r.points.toLocaleString()} points, ${r.width} × ${r.height} px, placed in project coordinates (recorded in the audit log).`,
                  );
                  setSlice(null);
                } catch (e) {
                  onNotice(String(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              {busy ? "Slicing…" : "Make slice"}
            </button>
          </div>
        </div>
      ) : (
        <button onClick={() => setSlice({ zMin: 0, zMax: 2, resolution: 0.01 })}>
          Add point-cloud slice…
        </button>
      )}
    </div>
  );
}
