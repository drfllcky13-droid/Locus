// Crash reconstruction, in the 3D view's side panel: speed from skid marks, critical speed
// from yaw marks, two-vehicle momentum and crush energy. Every input is a value with a
// tolerance; the backend gives the value, the range method's extremes and a Monte Carlo
// interval. Marks can be picked on the cloud (resolved again from stored data). Runs are saved
// as audit-logged analyses with PDF reports.
import { HelpButton, type Topic } from "../../help/Help";
import { useAllows } from "../../license";
import { useEffect, useState } from "react";
import * as THREE from "three";
import {
  api,
  type AnalysisRecord,
  type CrashInput,
  type CrashRecord,
  type CrashRequest,
  type CrashRun,
  type CrushProfile,
  type Spread,
  type StiffnessEntry,
} from "../../api";
import type { Engine } from "../../viewer3d/engine";
import type { PickHit } from "../../viewer3d/pointcloud";
import { RecordButtons } from "../camera/CameraPanel";

type Tool = "skid" | "yaw" | "momentum" | "crush" | "volume" | "edr";

/** A row of the EDR form, as typed. */
interface EdrRow {
  t: string;
  speed: string;
  accel: string;
  brake: "" | "on" | "off";
  steer: string;
}
/** The usual pre-crash table: 5 s before the trigger at 2 samples a second. */
const edrRows = (): EdrRow[] =>
  Array.from({ length: 11 }, (_, k) => ({
    t: (-5 + k * 0.5).toFixed(1),
    speed: "",
    accel: "",
    brake: "",
    steer: "",
  }));
type V3 = [number, number, number];

/** A value and a symmetric tolerance, as typed. */
interface Val {
  value: string;
  tol: string;
}
const val = (value: number | string, tol: number | string = 0): Val => ({
  value: String(value),
  tol: String(tol),
});
function toInput(v: Val): CrashInput | null {
  const [x, t] = [Number(v.value), Number(v.tol || 0)];
  if (v.value.trim() === "" || !Number.isFinite(x) || !Number.isFinite(t) || t < 0) return null;
  return { value: x, low: x - t, high: x + t };
}

function ValField({
  label,
  v,
  set,
  unit,
}: {
  label: string;
  v: Val;
  set: (v: Val) => void;
  unit?: string;
}) {
  return (
    <label className="val-field">
      {label}
      <span>
        <input
          className="narrow"
          inputMode="decimal"
          value={v.value}
          onChange={(e) => set({ ...v, value: e.target.value })}
        />
        ±
        <input
          className="narrow"
          inputMode="decimal"
          value={v.tol}
          onChange={(e) => set({ ...v, tol: e.target.value })}
        />
        {unit && <span className="muted"> {unit}</span>}
      </span>
    </label>
  );
}

interface Segment {
  label: string;
  distance: Val;
  drag: Val;
  braking: Val;
  grade: Val;
  path: PickHit[];
  points: V3[];
}

const newSegment = (k: number): Segment => ({
  label: `Stretch ${k}`,
  distance: val(""),
  drag: val(0.7, 0.05),
  braking: val(1),
  grade: val(0),
  path: [],
  points: [],
});

interface Vehicle {
  label: string;
  mass: Val;
  approach: Val;
  departure: Val;
  speed: Val;
}

const fmtSpeed = (s: Spread) =>
  `${s.value.toFixed(2)} m/s (${(s.value * 3.6).toFixed(1)} km/h); range ${s.low.toFixed(2)}–${s.high.toFixed(2)}; 95 % ${s.interval95[0].toFixed(2)}–${s.interval95[1].toFixed(2)} m/s`;

/** The skid marks and a yaw mark's points and fitted arc, in the 3D view. */
function crashOverlay(run: CrashRun, origin: V3): THREE.Group {
  const g = new THREE.Group();
  g.name = "crash";
  const rel = (p: V3) => new THREE.Vector3(p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]);
  const mat = new THREE.LineBasicMaterial({ color: 0x3fa9ff, depthTest: false });
  for (const s of run.segments ?? [])
    if (s.path.length >= 2) {
      const l = new THREE.Line(new THREE.BufferGeometry().setFromPoints(s.path.map(rel)), mat);
      l.renderOrder = 10;
      g.add(l);
    }
  if (run.radius_from?.kind === "points" && run.circle) {
    const c = run.circle;
    const pts = run.radius_from.points;
    const n = new THREE.Vector3(...c.normal);
    const e1 = new THREE.Vector3(...pts[0]).sub(new THREE.Vector3(...c.centre)).projectOnPlane(n);
    e1.normalize();
    const e2 = n.clone().cross(e1);
    const angle = (p: V3) => {
      const d = new THREE.Vector3(...p).sub(new THREE.Vector3(...c.centre));
      return Math.atan2(d.dot(e2), d.dot(e1));
    };
    const as = pts.map(angle);
    const [lo, hi] = [Math.min(...as), Math.max(...as)];
    const arc: THREE.Vector3[] = [];
    for (let k = 0; k <= 48; k++) {
      const t = lo + ((hi - lo) * k) / 48;
      const p = new THREE.Vector3(...c.centre)
        .addScaledVector(e1, c.radius * Math.cos(t))
        .addScaledVector(e2, c.radius * Math.sin(t));
      arc.push(rel([p.x, p.y, p.z]));
    }
    const l = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints(arc),
      new THREE.LineBasicMaterial({ color: 0xff453a, depthTest: false }),
    );
    l.renderOrder = 10;
    g.add(l);
    for (const p of pts) {
      const d = new THREE.Mesh(
        new THREE.SphereGeometry(0.03, 10, 8),
        new THREE.MeshBasicMaterial({ color: 0x3fa9ff, depthTest: false }),
      );
      d.position.copy(rel(p));
      d.renderOrder = 11;
      g.add(d);
    }
  }
  // Volumetric crush: the cells deeper than 2σ either way, on the face's plane, coloured by
  // depth (red in, blue out).
  if (run.result) {
    const v = run.result;
    const pos: number[] = [];
    const col: number[] = [];
    const scale = Math.max(v.max_depth, 0.01);
    const at = (s: number, t: number) =>
      [0, 1, 2].map((k) => v.origin[k] + v.u[k] * s + v.v[k] * t - origin[k]);
    for (const c of v.cells) {
      if (Math.abs(c.depth) <= 2 * c.sigma) continue;
      const [s, t, h] = [c.i * v.cell, c.j * v.cell, v.cell];
      const [a, b, d, e] = [at(s, t), at(s + h, t), at(s + h, t + h), at(s, t + h)];
      pos.push(...a, ...b, ...d, ...a, ...d, ...e);
      const x = Math.max(-1, Math.min(1, c.depth / scale));
      const rgb = x >= 0 ? [1, 1 - 0.8 * x, 1 - 0.85 * x] : [1 + 0.8 * x, 1 + 0.55 * x, 1];
      for (let k = 0; k < 6; k++) col.push(...rgb);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
    const m = new THREE.Mesh(
      geo,
      new THREE.MeshBasicMaterial({
        vertexColors: true,
        side: THREE.DoubleSide,
        depthTest: false,
        transparent: true,
        opacity: 0.8,
      }),
    );
    m.renderOrder = 10;
    g.add(m);
  }
  // EDR: the path, each sample's place on it, and the range of that place along the path.
  if (run.stations && run.path && run.path.length >= 2) {
    const l = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints(run.path.map(rel)),
      new THREE.LineBasicMaterial({ color: 0x3fa9ff, depthTest: false }),
    );
    l.renderOrder = 10;
    g.add(l);
    const spans: THREE.Vector3[] = [];
    for (const s of run.stations) {
      if (s.span) spans.push(rel(s.span[0]), rel(s.span[1]));
      if (!s.position) continue;
      const d = new THREE.Mesh(
        new THREE.SphereGeometry(0.15, 12, 8),
        new THREE.MeshBasicMaterial({ color: 0xff9f0a, depthTest: false }),
      );
      d.position.copy(rel(s.position));
      d.renderOrder = 11;
      g.add(d);
    }
    const sl = new THREE.LineSegments(
      new THREE.BufferGeometry().setFromPoints(spans),
      new THREE.LineBasicMaterial({ color: 0xff453a, depthTest: false }),
    );
    sl.renderOrder = 10;
    g.add(sl);
  }
  // A crush profile: the face line, and each station to the damaged surface behind it.
  if (run.profile) {
    const p = run.profile;
    const pts = [rel(p.start), rel(p.end)];
    for (const s of p.stations) pts.push(rel(s.at), rel(s.surface));
    const l = new THREE.LineSegments(
      new THREE.BufferGeometry().setFromPoints(pts),
      new THREE.LineBasicMaterial({ color: 0xff9f0a, depthTest: false }),
    );
    l.renderOrder = 10;
    g.add(l);
  }
  return g;
}

export function CrashPanel({
  engine,
  origin,
  requestPick,
  onNotice,
  scans,
}: {
  engine: () => Engine | null;
  origin: string;
  scans: { key: string; name: string }[];
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  onNotice: (m: string | null) => void;
}) {
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [tool, setTool] = useState<Tool | null>(null);
  const [name, setName] = useState("");
  const [segments, setSegments] = useState<Segment[]>([newSegment(1)]);
  const [endSpeed, setEndSpeed] = useState(val(0));
  const [yawMode, setYawMode] = useState<"chord" | "points">("chord");
  const [chord, setChord] = useState(val(""));
  const [ordinate, setOrdinate] = useState(val(""));
  const [yawPicks, setYawPicks] = useState<PickHit[]>([]);
  const [drag, setDrag] = useState(val(0.7, 0.05));
  const [superel, setSuperel] = useState(val(0));
  const [cgOffset, setCgOffset] = useState("0");
  const [vehicles, setVehicles] = useState<Vehicle[]>([
    { label: "Vehicle 1", mass: val(""), approach: val(""), departure: val(""), speed: val("") },
    { label: "Vehicle 2", mass: val(""), approach: val(""), departure: val(""), speed: val("") },
  ]);
  const [crushLabel, setCrushLabel] = useState("Front");
  const [a, setA] = useState(val(""));
  const [b, setB] = useState(val(""));
  const [source, setSource] = useState("");
  // Stiffness: from the NHTSA table (a vehicle chosen there) or entered with a source.
  const [fromTable, setFromTable] = useState(true);
  const [makes, setMakes] = useState<string[]>([]);
  const [make, setMake] = useState("");
  const [model, setModel] = useState("");
  const [year, setYear] = useState("");
  const [matches, setMatches] = useState<StiffnessEntry[]>([]);
  const [entry, setEntry] = useState<StiffnessEntry | null>(null);
  const [width, setWidth] = useState(val(""));
  const [depths, setDepths] = useState<Val[]>([val(""), val("")]);
  // Or the width and depths measured on the scan: the damage's two ends on the undamaged
  // face line and a point inside the vehicle, with the height band.
  const [measure, setMeasure] = useState(false);
  const [profilePicks, setProfilePicks] = useState<PickHit[]>([]);
  const [band, setBand] = useState("0.1");
  const [profile, setProfile] = useState<CrushProfile | null>(null);
  // Volumetric crush: the two scans, picked pairs (reference, damaged) and the damage region.
  const [volDamaged, setVolDamaged] = useState("");
  const [volReference, setVolReference] = useState("");
  // The reference: an exemplar's scan, or this vehicle's opposite side mirrored.
  const [volMirror, setVolMirror] = useState(false);
  const [volPairs, setVolPairs] = useState<[PickHit, PickHit][]>([]);
  const [region, setRegion] = useState<{ min: V3; max: V3 } | null>(null);
  const [cell, setCell] = useState("0.02");
  // EDR: the data (a CSV, or the form's rows), its source, the speed's accuracy and a path.
  const [edrLabel, setEdrLabel] = useState("Vehicle 1");
  const [edrSource, setEdrSource] = useState("");
  const [edrMode, setEdrMode] = useState<"csv" | "form">("form");
  const [edrCsv, setEdrCsv] = useState("");
  const [rows, setRows] = useState<EdrRow[]>(edrRows);
  const [unit, setUnit] = useState<"kmh" | "mph" | "ms">("kmh");
  const [scaleTol, setScaleTol] = useState("0");
  const [offsetTol, setOffsetTol] = useState("1");
  const [tolReason, setTolReason] = useState("");
  const [endTime, setEndTime] = useState("");
  const [edrPath, setEdrPath] = useState<PickHit[]>([]);
  const [pdof, setPdof] = useState(val(0));
  const [mass, setMass] = useState(val(""));
  const [result, setResult] = useState<CrashRun | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [shown, setShown] = useState<number | null>(null);

  useEffect(() => {
    api.analyses().then(setRecords, (e) => onNotice(String(e)));
  }, [onNotice]);
  // The stiffness table's makes, when the crush tool is first opened.
  useEffect(() => {
    if (tool === "crush" && makes.length === 0)
      api.stiffnessMakes().then(setMakes, (e) => onNotice(String(e)));
  }, [tool, makes.length, onNotice]);

  const all = <T,>(xs: (T | null)[]): T[] | null =>
    xs.every((x) => x !== null) ? (xs as T[]) : null;

  const request = ((): CrashRequest | null => {
    if (tool === "skid") {
      const segs = segments.map((s) => {
        const i = all([s.distance, s.drag, s.braking, s.grade].map(toInput));
        return (
          i && {
            label: s.label,
            distance: i[0],
            drag: i[1],
            braking: i[2],
            grade: i[3],
            path: s.path,
          }
        );
      });
      const e = toInput(endSpeed);
      return segs.every((s) => s) && e
        ? { tool, segments: segs.map((s) => s!), end_speed: e }
        : null;
    }
    if (tool === "yaw") {
      const [d, se] = [toInput(drag), toInput(superel)];
      const off = Number(cgOffset || 0);
      if (!d || !se || !(off >= 0)) return null;
      if (yawMode === "chord") {
        const [c, m] = [toInput(chord), toInput(ordinate)];
        return c && m
          ? { tool, chord: c, ordinate: m, points: [], drag: d, superelevation: se, cg_offset: off }
          : null;
      }
      return yawPicks.length >= 4
        ? {
            tool,
            chord: null,
            ordinate: null,
            points: yawPicks,
            drag: d,
            superelevation: se,
            cg_offset: off,
          }
        : null;
    }
    if (tool === "momentum") {
      const vs = vehicles.map((v) => {
        const i = all([v.mass, v.approach, v.departure, v.speed].map(toInput));
        return (
          i && {
            label: v.label,
            mass: i[0],
            approach_deg: i[1],
            departure_deg: i[2],
            departure_speed: i[3],
          }
        );
      });
      return vs.every((v) => v) ? { tool, vehicles: vs.map((v) => v!) } : null;
    }
    if (tool === "crush") {
      const i = all([pdof, mass].map(toInput));
      if (!i) return null;
      const base = { tool, label: crushLabel, pdof_deg: i[0], mass: i[1] };
      let rest;
      if (measure) {
        if (profilePicks.length !== 3 || !(Number(band) > 0)) return null;
        rest = {
          ...base,
          profile: { picks: profilePicks, stations: depths.length, band: Number(band) },
        };
      } else {
        const [w, ds] = [toInput(width), all(depths.map(toInput))];
        if (!w || !ds) return null;
        rest = { ...base, width: w, depths: ds, profile: null };
      }
      if (fromTable)
        return entry
          ? {
              ...rest,
              table: { make: entry.make, model: entry.model, model_year: entry.model_year },
            }
          : null;
      const [ai, bi] = [toInput(a), toInput(b)];
      return ai && bi && source.trim()
        ? { ...rest, table: null, a: ai, b: bi, stiffness_source: source }
        : null;
    }
    if (tool === "edr") {
      const unitName = { kmh: "km/h", mph: "mph", ms: "m/s" }[unit];
      const csv =
        edrMode === "csv"
          ? edrCsv
          : [
              `time (s),speed (${unitName}),accelerator (%),brake,steering (deg)`,
              ...rows
                .filter((r) => r.speed.trim() !== "")
                .map((r) => [r.t, r.speed, r.accel, r.brake, r.steer].join(",")),
            ].join("\n");
      const [st, ot] = [Number(scaleTol || 0) / 100, Number(offsetTol || 0)];
      const end = endTime.trim() === "" ? null : Number(endTime);
      return edrSource.trim() &&
        csv.trim() &&
        st >= 0 &&
        ot >= 0 &&
        (end === null || Number.isFinite(end))
        ? {
            tool,
            label: edrLabel,
            source: edrSource,
            csv,
            speed_unit: unit,
            scale_tolerance: st,
            offset_tolerance: ot / 3.6,
            tolerance_reason: tolReason,
            end_time: end,
            path: edrPath,
          }
        : null;
    }
    if (tool === "volume") {
      return volDamaged &&
        (volMirror || (volReference && volDamaged !== volReference)) &&
        volPairs.length >= 3 &&
        region &&
        Number(cell) > 0
        ? {
            tool,
            label: crushLabel,
            damaged: volDamaged,
            reference: volMirror ? volDamaged : volReference,
            mirror: volMirror,
            pairs: volPairs,
            lo: region.min,
            hi: region.max,
            cell: Number(cell),
          }
        : null;
    }
    return null;
  })();
  const reqKey = JSON.stringify(request);
  useEffect(() => {
    if (!request) return;
    let live = true;
    const t = setTimeout(
      () =>
        api.crashPreview(request).then(
          (r) => live && (setResult(r), setFailure(null)),
          (e) => live && (setResult(null), setFailure(String(e))),
        ),
      300,
    );
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reqKey stands for request
  }, [reqKey]);
  const preview = tool && request ? result : null;
  const crashes = records.filter((r): r is CrashRecord =>
    ["skid", "yaw", "momentum", "crush", "crush_volume", "edr"].includes(r.tool),
  );
  const drawn = tool ? preview : (crashes.find((r) => r.id === shown)?.record ?? null);

  useEffect(() => {
    const e = engine();
    if (e) e.setAnalysisOverlay("crash", drawn ? crashOverlay(drawn, e.origin) : null);
  }, [drawn, engine, origin]);
  useEffect(() => () => engine()?.setAnalysisOverlay("crash", null), [engine]);

  const pickMark = (i: number) =>
    requestPick("Click the next point along the mark.", (hit) => {
      const path = [...segments[i].path, hit];
      setSegments((ss) => ss.map((s, j) => (j === i ? { ...s, path } : s)));
      if (path.length >= 2)
        api.crashMarkLength(path).then(
          (m) =>
            setSegments((ss) =>
              ss.map((s, j) =>
                j === i
                  ? {
                      ...s,
                      path,
                      points: m.points,
                      distance: { value: m.length.toFixed(4), tol: s.distance.tol || "0.05" },
                    }
                  : s,
              ),
            ),
          (e) => onNotice(String(e)),
        );
    });

  const save = async () => {
    if (!request) return;
    try {
      const rec = await api.crashSave(name || `${tool} analysis`, request, null);
      setRecords(await api.analyses());
      setTool(null);
      setShown(rec.id);
      onNotice(`Saved as analysis ${rec.id}: ${rec.record.summary} (recorded in the audit log).`);
    } catch (e) {
      onNotice(String(e));
    }
  };

  const crushVolume = useAllows("crush_volume");
  // Tools with a method note of their own, besides the crash note in the heading.
  const TOOL_NOTE: Partial<Record<Tool, Topic>> = {
    crush: "crash-stiffness",
    volume: "crash-volume",
    edr: "crash-edr",
  };
  const titles: Record<Tool, string> = {
    skid: "Speed from skid marks",
    yaw: "Critical speed from a yaw mark",
    momentum: "Linear momentum (two vehicles)",
    crush: "Crush energy (CRASH3)",
    volume: "Crush volume (against a reference scan)",
    edr: "EDR pre-crash data",
  };
  return (
    <section className="panel-section crash">
      <h3 className="with-help">
        Crash reconstruction <HelpButton topic="crash" />
      </h3>
      {!tool && (
        <div className="buttons">
          {(Object.keys(titles) as Tool[])
            .filter((t) => t !== "volume" || crushVolume)
            .map((t) => (
              <button
                key={t}
                onClick={() => {
                  setTool(t);
                  const stored = t === "volume" ? "crush_volume" : t;
                  setName(`${titles[t]} ${crashes.filter((r) => r.tool === stored).length + 1}`);
                  setResult(null);
                }}
              >
                {titles[t]}…
              </button>
            ))}
        </div>
      )}
      {tool && (
        <div className="dg-built">
          <strong className="with-help">
            {titles[tool]} {TOOL_NOTE[tool] && <HelpButton topic={TOOL_NOTE[tool]} />}
          </strong>
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <p className="muted">Each value has a ± tolerance: the range it could be in.</p>
          {tool === "skid" && (
            <>
              {segments.map((s, i) => {
                const set = (p: Partial<Segment>) =>
                  setSegments((ss) => ss.map((x, j) => (j === i ? { ...x, ...p } : x)));
                return (
                  <fieldset key={i}>
                    <input value={s.label} onChange={(e) => set({ label: e.target.value })} />
                    <ValField
                      label="Length"
                      v={s.distance}
                      set={(distance) => set({ distance, path: [], points: [] })}
                      unit="m"
                    />
                    <div className="buttons">
                      <button onClick={() => pickMark(i)}>
                        {s.path.length
                          ? `Add a point (${s.path.length} picked)`
                          : "Measure on the cloud"}
                      </button>
                      {s.path.length > 0 && (
                        <button onClick={() => set({ path: [], points: [] })}>Clear</button>
                      )}
                    </div>
                    <ValField label="Drag factor μ" v={s.drag} set={(drag) => set({ drag })} />
                    <ValField
                      label="Braking efficiency n"
                      v={s.braking}
                      set={(braking) => set({ braking })}
                    />
                    <ValField
                      label="Grade (rise/run, + uphill)"
                      v={s.grade}
                      set={(grade) => set({ grade })}
                    />
                    {segments.length > 1 && (
                      <button onClick={() => setSegments((ss) => ss.filter((_, j) => j !== i))}>
                        Remove
                      </button>
                    )}
                  </fieldset>
                );
              })}
              <button onClick={() => setSegments((ss) => [...ss, newSegment(ss.length + 1)])}>
                Add a surface
              </button>
              <ValField
                label="Speed at the end of the marks"
                v={endSpeed}
                set={setEndSpeed}
                unit="m/s"
              />
            </>
          )}
          {tool === "yaw" && (
            <>
              <label>
                Radius from
                <select
                  value={yawMode}
                  onChange={(e) => setYawMode(e.target.value as "chord" | "points")}
                >
                  <option value="chord">a chord and middle ordinate</option>
                  <option value="points">points picked along the mark</option>
                </select>
              </label>
              {yawMode === "chord" ? (
                <>
                  <ValField label="Chord" v={chord} set={setChord} unit="m" />
                  <ValField label="Middle ordinate" v={ordinate} set={setOrdinate} unit="m" />
                </>
              ) : (
                <div className="buttons">
                  <button
                    onClick={() =>
                      requestPick(
                        "Click the next point along the yaw mark (early in the mark).",
                        (h) => setYawPicks((ps) => [...ps, h]),
                      )
                    }
                  >
                    Pick a point ({yawPicks.length}; at least 4)
                  </button>
                  {yawPicks.length > 0 && <button onClick={() => setYawPicks([])}>Clear</button>}
                </div>
              )}
              <ValField label="Drag factor μ" v={drag} set={setDrag} />
              <ValField label="Superelevation (toward the centre)" v={superel} set={setSuperel} />
              <label>
                Mark to the centre of mass&apos;s path (m, half the track for the outside front
                tyre)
                <input
                  className="narrow"
                  inputMode="decimal"
                  value={cgOffset}
                  onChange={(e) => setCgOffset(e.target.value)}
                />
              </label>
            </>
          )}
          {tool === "momentum" &&
            vehicles.map((v, i) => {
              const set = (p: Partial<Vehicle>) =>
                setVehicles((vs) => vs.map((x, j) => (j === i ? { ...x, ...p } : x)));
              return (
                <fieldset key={i}>
                  <input value={v.label} onChange={(e) => set({ label: e.target.value })} />
                  <ValField label="Mass" v={v.mass} set={(mass) => set({ mass })} unit="kg" />
                  <ValField
                    label="Approach direction (° from north)"
                    v={v.approach}
                    set={(approach) => set({ approach })}
                  />
                  <ValField
                    label="Departure direction (° from north)"
                    v={v.departure}
                    set={(departure) => set({ departure })}
                  />
                  <ValField
                    label="Departure speed"
                    v={v.speed}
                    set={(speed) => set({ speed })}
                    unit="m/s"
                  />
                </fieldset>
              );
            })}
          {tool === "crush" && (
            <>
              <label>
                Face
                <input value={crushLabel} onChange={(e) => setCrushLabel(e.target.value)} />
              </label>
              <label>
                Stiffness A and B
                <select
                  value={fromTable ? "table" : "entered"}
                  onChange={(e) => {
                    setFromTable(e.target.value === "table");
                  }}
                >
                  <option value="table">from NHTSA frontal barrier tests</option>
                  <option value="entered">entered, with their source</option>
                </select>
              </label>
              {fromTable ? (
                <>
                  <div className="buttons">
                    <select value={make} onChange={(e) => setMake(e.target.value)}>
                      <option value="">Make…</option>
                      {makes.map((m) => (
                        <option key={m}>{m}</option>
                      ))}
                    </select>
                    <input
                      className="narrow"
                      placeholder="Model"
                      value={model}
                      onChange={(e) => setModel(e.target.value)}
                    />
                    <input
                      className="narrow"
                      placeholder="Year"
                      inputMode="numeric"
                      value={year}
                      onChange={(e) => setYear(e.target.value)}
                    />
                    <button
                      disabled={!make}
                      onClick={() => {
                        const y = Number(year) || null;
                        api
                          .stiffnessLookup(make, model, y && y - 2, y && y + 2)
                          .then(setMatches, (err) => onNotice(String(err)));
                      }}
                    >
                      Find
                    </button>
                  </div>
                  {matches.length > 0 && (
                    <label>
                      Vehicle
                      <select
                        value={entry ? `${entry.make}|${entry.model}|${entry.model_year}` : ""}
                        onChange={(e) =>
                          setEntry(
                            matches.find(
                              (m) => `${m.make}|${m.model}|${m.model_year}` === e.target.value,
                            ) ?? null,
                          )
                        }
                      >
                        <option value="">Choose…</option>
                        {matches.map((m) => (
                          <option
                            key={`${m.make}|${m.model}|${m.model_year}`}
                            value={`${m.make}|${m.model}|${m.model_year}`}
                          >
                            {m.model_year} {m.model} ({m.tests.length} test
                            {m.tests.length > 1 ? "s" : ""})
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                  {entry && (
                    <p className={entry.single_test ? "error" : "muted"}>
                      A {entry.a.toFixed(0)} ± {entry.a_sigma.toFixed(0)} N/m, B{" "}
                      {entry.b.toFixed(0)} ± {entry.b_sigma.toFixed(0)} N/m² (1σ), from NHTSA test
                      {entry.tests.length > 1 ? "s" : ""}{" "}
                      {entry.tests.map((t) => t.test_no).join(", ")}
                      {entry.single_test ? ". One test only: its spread can't be known." : "."}
                    </p>
                  )}
                </>
              ) : (
                <>
                  <ValField label="A" v={a} set={setA} unit="N/m" />
                  <ValField label="B" v={b} set={setB} unit="N/m²" />
                  <label>
                    Source of A and B (in the report; required)
                    <input value={source} onChange={(e) => setSource(e.target.value)} />
                  </label>
                </>
              )}
              <label>
                Width and depths
                <select
                  value={measure ? "scan" : "entered"}
                  onChange={(e) => setMeasure(e.target.value === "scan")}
                >
                  <option value="entered">entered</option>
                  <option value="scan">measured on the scan</option>
                </select>
              </label>
              {!measure && <ValField label="Damage width" v={width} set={setWidth} unit="m" />}
              <label>
                Crush depths
                <select
                  value={depths.length}
                  onChange={(e) =>
                    setDepths((ds) =>
                      Array.from({ length: Number(e.target.value) }, (_, k) => ds[k] ?? val("")),
                    )
                  }
                >
                  {[2, 4, 6].map((n) => (
                    <option key={n} value={n}>
                      {n} points
                    </option>
                  ))}
                </select>
              </label>
              {!measure &&
                depths.map((d, k) => (
                  <ValField
                    key={k}
                    label={`C${k + 1}`}
                    v={d}
                    set={(x) => setDepths((ds) => ds.map((y, j) => (j === k ? x : y)))}
                    unit="m"
                  />
                ))}
              {measure && (
                <>
                  <label>
                    Height band (± m around the ends&apos; height)
                    <input
                      className="narrow"
                      inputMode="decimal"
                      value={band}
                      onChange={(e) => setBand(e.target.value)}
                    />
                  </label>
                  <div className="buttons">
                    <button
                      onClick={() => {
                        setProfile(null);
                        const hints = [
                          "Click one end of the damage on the undamaged face line, at the measuring height.",
                          "Click the other end of the damage on the face line.",
                          "Click any point inside the vehicle, behind the damaged face.",
                        ];
                        const next = (picked: PickHit[]) =>
                          requestPick(hints[picked.length], (h) => {
                            const all3 = [...picked, h];
                            setProfilePicks(all3);
                            if (all3.length < 3) next(all3);
                            else
                              api
                                .crashCrushProfile({
                                  picks: all3,
                                  stations: depths.length,
                                  band: Number(band),
                                })
                                .then(setProfile, (err) => onNotice(String(err)));
                          });
                        setProfilePicks([]);
                        next([]);
                      }}
                    >
                      {profilePicks.length === 3
                        ? "Pick again"
                        : "Pick the ends and a point inside"}
                    </button>
                  </div>
                  {profile && (
                    <p className="muted">
                      Width {profile.width.toFixed(3)} m at {profile.height.toFixed(2)} m; depths{" "}
                      {profile.stations
                        .map(
                          (s, k) =>
                            `C${k + 1} ${(s.depth * 1000).toFixed(0)} ± ${(s.sigma * 1000).toFixed(0)} mm`,
                        )
                        .join(", ")}
                    </p>
                  )}
                </>
              )}
              <ValField
                label="Principal direction of force (° off normal)"
                v={pdof}
                set={setPdof}
              />
              <ValField label="Mass" v={mass} set={setMass} unit="kg" />
            </>
          )}
          {tool === "edr" && (
            <>
              <label>
                Vehicle
                <input value={edrLabel} onChange={(e) => setEdrLabel(e.target.value)} />
              </label>
              <label>
                Source (in the report; required)
                <input
                  value={edrSource}
                  placeholder="Retrieval report, VIN, retrieved by, date"
                  onChange={(e) => setEdrSource(e.target.value)}
                />
              </label>
              <label>
                Data
                <select
                  value={edrMode}
                  onChange={(e) => setEdrMode(e.target.value as "csv" | "form")}
                >
                  <option value="form">entered from the pre-crash table</option>
                  <option value="csv">imported from a CSV file</option>
                </select>
              </label>
              <label>
                Speed unit{edrMode === "csv" ? " (where a header gives none)" : ""}
                <select
                  value={unit}
                  onChange={(e) => setUnit(e.target.value as "kmh" | "mph" | "ms")}
                >
                  <option value="kmh">km/h</option>
                  <option value="mph">mph</option>
                  <option value="ms">m/s</option>
                </select>
              </label>
              {edrMode === "csv" && (
                <>
                  <input
                    type="file"
                    accept=".csv,.txt"
                    onChange={(e) => {
                      const f = e.target.files?.[0];
                      if (f) void f.text().then(setEdrCsv);
                    }}
                  />
                  <textarea
                    rows={6}
                    value={edrCsv}
                    placeholder="Time (s), Speed (km/h), Accelerator (%), Brake, Steering (deg)"
                    onChange={(e) => setEdrCsv(e.target.value)}
                  />
                </>
              )}
              {edrMode === "form" && (
                <table className="edr-form">
                  <thead>
                    <tr>
                      <th>t (s)</th>
                      <th>Speed</th>
                      <th>Accel. %</th>
                      <th>Brake</th>
                      <th>Steer °</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r, k) => {
                      const set = (p: Partial<EdrRow>) =>
                        setRows((rs) => rs.map((x, j) => (j === k ? { ...x, ...p } : x)));
                      return (
                        <tr key={k}>
                          <td>
                            <input
                              className="narrow"
                              value={r.t}
                              onChange={(e) => set({ t: e.target.value })}
                            />
                          </td>
                          <td>
                            <input
                              className="narrow"
                              value={r.speed}
                              onChange={(e) => set({ speed: e.target.value })}
                            />
                          </td>
                          <td>
                            <input
                              className="narrow"
                              value={r.accel}
                              onChange={(e) => set({ accel: e.target.value })}
                            />
                          </td>
                          <td>
                            <select
                              value={r.brake}
                              onChange={(e) => set({ brake: e.target.value as EdrRow["brake"] })}
                            >
                              <option value=""></option>
                              <option value="on">on</option>
                              <option value="off">off</option>
                            </select>
                          </td>
                          <td>
                            <input
                              className="narrow"
                              value={r.steer}
                              onChange={(e) => set({ steer: e.target.value })}
                            />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}
              {edrMode === "form" && (
                <div className="buttons">
                  <button
                    onClick={() =>
                      setRows((rs) => [
                        ...rs,
                        {
                          t: (Number(rs[rs.length - 1]?.t ?? -0.5) + 0.5).toFixed(1),
                          speed: "",
                          accel: "",
                          brake: "",
                          steer: "",
                        },
                      ])
                    }
                  >
                    Add a row
                  </button>
                  <button onClick={() => setRows((rs) => rs.slice(0, -1))}>
                    Remove the last row
                  </button>
                </div>
              )}
              <label>
                Speed tolerance ± %
                <input
                  className="narrow"
                  inputMode="decimal"
                  value={scaleTol}
                  onChange={(e) => setScaleTol(e.target.value)}
                />
              </label>
              <label>
                and ± km/h (at least 1, the recording accuracy)
                <input
                  className="narrow"
                  inputMode="decimal"
                  value={offsetTol}
                  onChange={(e) => setOffsetTol(e.target.value)}
                />
              </label>
              {(Number(scaleTol || 0) > 0 || Number(offsetTol || 0) > 1) && (
                <label>
                  Why wider than ±1 km/h (in the report; required)
                  <input
                    value={tolReason}
                    placeholder="Wheel slip under braking, ABS, non-original tyres…"
                    onChange={(e) => setTolReason(e.target.value)}
                  />
                </label>
              )}
              <p className="muted">
                ±1 km/h is the recording accuracy of the indicated speed, not of the speed over the
                ground: wheel slip, ABS cycling and non-original tyres can make them differ.
              </p>
              <label>
                End time (s; blank for the last sample)
                <input
                  className="narrow"
                  inputMode="decimal"
                  value={endTime}
                  onChange={(e) => setEndTime(e.target.value)}
                />
              </label>
              <p className="muted">
                Optionally, pick the path the vehicle took, in order, ending where its reference
                point was at the end time.
              </p>
              <div className="buttons">
                <button
                  onClick={() =>
                    requestPick(
                      "Click the next point along the path (the last at the end time).",
                      (h) => setEdrPath((p) => [...p, h]),
                    )
                  }
                >
                  {edrPath.length ? `Add a path point (${edrPath.length})` : "Pick the path"}
                </button>
                {edrPath.length > 0 && (
                  <button onClick={() => setEdrPath([])}>Clear the path</button>
                )}
              </div>
            </>
          )}
          {tool === "volume" && (
            <>
              <label>
                Face
                <input value={crushLabel} onChange={(e) => setCrushLabel(e.target.value)} />
              </label>
              <label>
                Damaged vehicle&apos;s scan
                <select value={volDamaged} onChange={(e) => setVolDamaged(e.target.value)}>
                  <option value="">Choose…</option>
                  {scans.map((s) => (
                    <option key={s.key} value={s.key}>
                      {s.name}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Reference
                <select
                  value={volMirror ? "mirror" : "exemplar"}
                  onChange={(e) => {
                    setVolMirror(e.target.value === "mirror");
                    setVolPairs([]);
                  }}
                >
                  <option value="exemplar">an undamaged vehicle&apos;s scan (exemplar)</option>
                  <option value="mirror">this vehicle&apos;s opposite side, mirrored</option>
                </select>
              </label>
              {!volMirror && (
                <label>
                  Reference (undamaged) scan
                  <select value={volReference} onChange={(e) => setVolReference(e.target.value)}>
                    <option value="">Choose…</option>
                    {scans.map((s) => (
                      <option key={s.key} value={s.key}>
                        {s.name}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <p className="muted">
                {volMirror
                  ? "Pick at least 3 pairs of symmetric features away from the damage (mirror bases, lamp corners, wheel centres): each on the left, then its counterpart on the right. Real vehicles aren't exactly symmetric; the asymmetry measured on the undamaged surfaces goes into the interval."
                  : "Pick at least 3 undamaged features away from the damage, each on the reference and then on the damaged vehicle."}
              </p>
              <div className="buttons">
                <button
                  onClick={() =>
                    volMirror
                      ? requestPick("Click a symmetric feature on the left side.", (r) =>
                          requestPick("Click its counterpart on the right side.", (d) =>
                            setVolPairs((ps) => [...ps, [r, d]]),
                          ),
                        )
                      : requestPick("Click an undamaged feature on the reference.", (r) =>
                          requestPick("Click the same feature on the damaged vehicle.", (d) =>
                            setVolPairs((ps) => [...ps, [r, d]]),
                          ),
                        )
                  }
                >
                  Pick a pair ({volPairs.length})
                </button>
                {volPairs.length > 0 && (
                  <button onClick={() => setVolPairs([])}>Clear pairs</button>
                )}
              </div>
              <div className="buttons">
                <button
                  onClick={() => {
                    const r = engine()?.clipRegion();
                    if (r) setRegion({ min: r.min as V3, max: r.max as V3 });
                    else onNotice("Turn the clip box on and fit it around the damage first.");
                  }}
                >
                  Use the clip box as the damage region
                </button>
              </div>
              {region && (
                <p className="muted">
                  Region {region.max.map((h, k) => (h - region.min[k]).toFixed(2)).join(" × ")} m
                </p>
              )}
              <label>
                Cell size
                <input
                  className="narrow"
                  inputMode="decimal"
                  value={cell}
                  onChange={(e) => setCell(e.target.value)}
                />{" "}
                m
              </label>
            </>
          )}
          {failure && request && <p className="error">{failure}</p>}
          {preview && (
            <div className="trajectory-result">
              {preview.speed && (
                <div>
                  {tool === "yaw" ? "Critical speed" : "Speed"}:{" "}
                  <strong>{fmtSpeed(preview.speed)}</strong>
                </div>
              )}
              {preview.radius && (
                <div className="muted">Radius {preview.radius.value.toFixed(2)} m</div>
              )}
              {preview.speeds?.map((s, k) => (
                <div key={k}>
                  {vehicles[k].label}: <strong>{fmtSpeed(s)}</strong>
                </div>
              ))}
              {preview.energy && (
                <div>
                  Energy {(preview.energy.value / 1000).toFixed(1)} kJ; equivalent barrier speed{" "}
                  <strong>{fmtSpeed(preview.ebs!)}</strong>
                </div>
              )}
              {preview.stations && preview.stations.length > 0 && (
                <div>
                  Distance from {preview.stations[0].t.toFixed(1)} s to the end:{" "}
                  <strong>{preview.stations[0].distance.value.toFixed(2)} m</strong> (range{" "}
                  {preview.stations[0].distance.low.toFixed(2)}–
                  {preview.stations[0].distance.high.toFixed(2)} m)
                </div>
              )}
              {preview.result && (
                <div>
                  Crush volume <strong>{(preview.result.inward.value * 1000).toFixed(2)} L</strong>{" "}
                  (95 % {(preview.result.inward.interval95[0] * 1000).toFixed(2)}–
                  {(preview.result.inward.interval95[1] * 1000).toFixed(2)} L); pushed outward{" "}
                  {(preview.result.outward * 1000).toFixed(2)} L; deepest{" "}
                  {(preview.result.max_depth * 1000).toFixed(0)} mm
                  {preview.registration && (
                    <div className="muted">
                      Registration: pairs {(preview.registration.pairs_rms * 1000).toFixed(1)} mm
                      RMS, ICP {(preview.registration.icp_rms * 1000).toFixed(1)} mm RMS,{" "}
                      {(preview.registration.overlap * 100).toFixed(0)} % overlap; 1σ{" "}
                      {(preview.registration.sigma_translation * 1000).toFixed(1)} mm,{" "}
                      {preview.registration.sigma_rotation_deg.toFixed(3)}°
                    </div>
                  )}
                </div>
              )}
              {preview.warnings?.map((w, i) => (
                <p key={i} className="error">
                  {w}
                </p>
              ))}
            </div>
          )}
          <div className="buttons">
            <button onClick={() => setTool(null)}>Cancel</button>
            <button className="primary" disabled={!preview} onClick={() => void save()}>
              Save analysis
            </button>
          </div>
        </div>
      )}
      {crashes.map((r) => (
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
