import { useState } from "react";
import { api, type HandRequest, type HandSolved } from "../api";
import type { Entity, EntityInput, FieldMeasurement } from "./model";

type PointEntity = Extract<Entity, { kind: "point" }>;
type Side = "Left" | "Right";

const describe = (p: PointEntity) =>
  `${p.label || "unnamed"} (${p.at[0].toFixed(3)}, ${p.at[1].toFixed(3)})`;

/**
 * Place a point from tape measurements: baseline/offset, or triangulation from two or more
 * points already on the diagram. The raw readings are kept with the point.
 */
export function MeasureDialog({
  points,
  onPlace,
  onClose,
}: {
  points: PointEntity[];
  onPlace: (e: EntityInput) => void;
  onClose: () => void;
}) {
  const [method, setMethod] = useState<FieldMeasurement["method"]>("triangulation");
  const [label, setLabel] = useState("");
  const [from, setFrom] = useState(points[0]?.id ?? "");
  const [to, setTo] = useState(points[1]?.id ?? "");
  const [along, setAlong] = useState("");
  const [offset, setOffset] = useState("");
  const [side, setSide] = useState<Side | "">("Left");
  const [refs, setRefs] = useState<{ point: string; distance: string }[]>(
    points.slice(0, 2).map((p) => ({ point: p.id, distance: "" })),
  );
  // Precision, in the UI's millimetres.
  const [tapeMm, setTapeMm] = useState(2);
  const [tapePerM, setTapePerM] = useState(1);
  const [refMm, setRefMm] = useState(0);
  const [result, setResult] = useState<HandSolved | null>(null);
  const [error, setError] = useState<string | null>(null);

  const byId = (id: string) => points.find((p) => p.id === id);
  const num = (s: string) => (s.trim() === "" ? NaN : Number(s));

  const request = (): { req: HandRequest; record: FieldMeasurement } | string => {
    if (method === "baseline_offset") {
      const a = byId(from);
      const b = byId(to);
      if (!a || !b || a === b) return "Choose two different baseline points.";
      if (!Number.isFinite(num(along)) || !Number.isFinite(num(offset)))
        return "Enter the distance along the baseline and the offset, in metres.";
      const s: Side = side || "Left";
      return {
        req: { method, from: a.at, to: b.at, along: num(along), offset: num(offset), side: s },
        record: { method, from: a.id, to: b.id, along: num(along), offset: num(offset), side: s },
      };
    }
    const rows = refs.map((r) => ({ p: byId(r.point), d: num(r.distance), r }));
    if (rows.length < 2 || rows.some((x) => !x.p || !Number.isFinite(x.d) || x.d <= 0))
      return "Choose at least two reference points and enter each distance in metres.";
    if (new Set(rows.map((x) => x.r.point)).size !== rows.length)
      return "Each reference point can be used once.";
    return {
      req: { method, refs: rows.map((x) => [x.p!.at, x.d]), side: side || null },
      record: {
        method,
        refs: rows.map((x) => ({ point: x.r.point, distance: x.d })),
        side: side || null,
      },
    };
  };

  const solve = async () => {
    setResult(null);
    const r = request();
    if (typeof r === "string") {
      setError(r);
      return;
    }
    try {
      setResult(await api.handSolve(r.req, refMm / 1000, tapeMm / 1000, tapePerM / 1000));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const place = () => {
    const r = request();
    if (!result || typeof r === "string") return;
    const sigma = Math.sqrt(result.covariance[0][0] + result.covariance[1][1]);
    onPlace({
      kind: "point",
      at: result.position,
      label: label.trim(),
      measurement: r.record,
      sigma,
    });
    onClose();
  };

  const pointSelect = (value: string, set: (v: string) => void, label: string) => (
    <select
      value={value}
      onChange={(e) => {
        set(e.target.value);
        setResult(null);
      }}
      aria-label={label}
    >
      {points.map((p) => (
        <option key={p.id} value={p.id}>
          {describe(p)}
        </option>
      ))}
    </select>
  );

  const sigma = result ? Math.sqrt(result.covariance[0][0] + result.covariance[1][1]) : 0;

  return (
    <div
      className="dg-form dg-measure"
      onPointerDown={(e) => e.stopPropagation()}
      onPointerUp={(e) => e.stopPropagation()}
    >
      <strong>Measured point</strong>
      {points.length < 2 ? (
        <p className="warn">
          Place at least two reference points first (Point tool, with their coordinates).
        </p>
      ) : (
        <>
          <label>
            Method
            <select
              value={method}
              onChange={(e) => {
                setMethod(e.target.value as FieldMeasurement["method"]);
                setResult(null);
              }}
            >
              <option value="triangulation">Triangulation (distances from known points)</option>
              <option value="baseline_offset">Baseline and offset</option>
            </select>
          </label>
          {method === "baseline_offset" ? (
            <>
              <label>Baseline from {pointSelect(from, setFrom, "Baseline start")}</label>
              <label>to {pointSelect(to, setTo, "Baseline end")}</label>
              <label>
                Along the baseline (m)
                <input
                  value={along}
                  onChange={(e) => {
                    setAlong(e.target.value);
                    setResult(null);
                  }}
                />
              </label>
              <label>
                Offset, square to it (m)
                <input
                  value={offset}
                  onChange={(e) => {
                    setOffset(e.target.value);
                    setResult(null);
                  }}
                />
              </label>
            </>
          ) : (
            <>
              {refs.map((r, i) => (
                <div key={i} className="dg-ref">
                  {pointSelect(
                    r.point,
                    (v) => setRefs(refs.map((x, j) => (j === i ? { ...x, point: v } : x))),
                    `Reference ${i + 1}`,
                  )}
                  <input
                    aria-label={`Distance from reference ${i + 1} (m)`}
                    placeholder="m"
                    value={r.distance}
                    onChange={(e) => {
                      setRefs(
                        refs.map((x, j) => (j === i ? { ...x, distance: e.target.value } : x)),
                      );
                      setResult(null);
                    }}
                  />
                  {refs.length > 2 && (
                    <button
                      onClick={() => setRefs(refs.filter((_, j) => j !== i))}
                      aria-label="Remove reference"
                    >
                      ×
                    </button>
                  )}
                </div>
              ))}
              {refs.length < points.length && (
                <button
                  onClick={() =>
                    setRefs([
                      ...refs,
                      { point: points[refs.length]?.id ?? points[0].id, distance: "" },
                    ])
                  }
                >
                  Add a reference
                </button>
              )}
            </>
          )}
          <label>
            Side of the line from the first point to the second
            <select
              value={side}
              onChange={(e) => {
                setSide(e.target.value as Side | "");
                setResult(null);
              }}
            >
              {method === "triangulation" && (
                <option value="">Decide from three or more distances</option>
              )}
              <option value="Left">Left</option>
              <option value="Right">Right</option>
            </select>
          </label>
          <div className="dg-precision">
            <label>
              Tape ± (mm)
              <input
                type="number"
                value={tapeMm}
                min={0}
                step={0.5}
                onChange={(e) => setTapeMm(+e.target.value)}
              />
            </label>
            <label>
              + per metre (mm)
              <input
                type="number"
                value={tapePerM}
                min={0}
                step={0.1}
                onChange={(e) => setTapePerM(+e.target.value)}
              />
            </label>
            <label>
              Reference points ± (mm)
              <input
                type="number"
                value={refMm}
                min={0}
                step={0.5}
                onChange={(e) => setRefMm(+e.target.value)}
              />
            </label>
          </div>
          <label>
            Label
            <input value={label} onChange={(e) => setLabel(e.target.value)} />
          </label>
          {error && <p className="warn">{error}</p>}
          {result && (
            <div className="dg-result">
              <div>
                x {result.position[0].toFixed(3)} m, y {result.position[1].toFixed(3)} m, ±{" "}
                {(sigma * 1000).toFixed(1)} mm (1σ)
              </div>
              {result.residuals.length > 0 && (
                <div className="muted">
                  Residuals: {result.residuals.map((r) => `${(r * 1000).toFixed(1)} mm`).join(", ")}
                </div>
              )}
              {result.worst_normalised !== null && result.worst_normalised > 3 && (
                <div className="warn">
                  The tapes disagree beyond their stated precision (worst{" "}
                  {result.worst_normalised.toFixed(1)}σ). Check the readings.
                </div>
              )}
            </div>
          )}
        </>
      )}
      <div className="buttons">
        <button onClick={onClose}>Cancel</button>
        {points.length >= 2 && <button onClick={() => void solve()}>Solve</button>}
        <button className="primary" disabled={!result} onClick={place}>
          Place point
        </button>
      </div>
    </div>
  );
}
