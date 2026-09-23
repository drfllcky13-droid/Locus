import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, type DiagramRevision } from "../api";
import { dist, endpoints, foot, segments, snap, type Snap } from "./geometry";
import { MeasureDialog } from "./MeasureDialog";
import {
  legendItems,
  nextMarker,
  renumberMarkers,
  SYMBOLS,
  symbolById,
  type LegendItem,
} from "./symbols";
import type { Diagram, Entity, EntityInput, Layer, Pt } from "./model";
import {
  commit,
  gridStep,
  history,
  pan,
  redo,
  toScreen,
  toWorld,
  undo,
  zoomAt,
  type View,
} from "./view";

type Tool =
  | "select"
  | "line"
  | "polyline"
  | "arc"
  | "dimension"
  | "text"
  | "point"
  | "measure"
  | "symbol"
  | "marker"
  | "north"
  | "scalebar"
  | "legend";

const TOOLS: [Tool, string][] = [
  ["select", "Select"],
  ["line", "Line"],
  ["polyline", "Polyline"],
  ["arc", "Arc"],
  ["dimension", "Dimension"],
  ["text", "Text"],
  ["point", "Point"],
  ["measure", "Measured point"],
  ["north", "North arrow"],
  ["symbol", "Symbol"],
  ["marker", "Evidence marker"],
  ["scalebar", "Scale bar"],
  ["legend", "Legend"],
];

/** Instructions for each click of each tool. */
const STEPS: Record<Tool, string[]> = {
  select: ["Click an item to select it; Delete removes it. Drag to pan, wheel to zoom."],
  line: ["Click the start.", "Click the end (Esc to stop)."],
  polyline: ["Click the first point.", "Click the next point; Enter or double-click to finish."],
  arc: [
    "Click the centre.",
    "Click the start (sets the radius).",
    "Click the end (anticlockwise).",
  ],
  dimension: [
    "Click the first point.",
    "Click the second point.",
    "Click where the dimension line goes.",
  ],
  text: ["Click where the text goes."],
  point: ["Click to place a point, or type its coordinates below."],
  measure: ["Enter the tape measurements in the panel."],
  north: ["Click where the north arrow goes."],
  scalebar: ["Click where the scale bar starts."],
  symbol: ["Pick a symbol on the right, then click to place it."],
  marker: ["Click to place the next evidence marker."],
  legend: ["Click where the legend goes; it lists what the diagram uses."],
};

const newId = () => crypto.randomUUID();
const fmt = (m: number) => `${m.toFixed(3)} m`;
const AUTOSAVE_MS = 1500;

/** Distance from `p` to an entity, for picking (m). */
function distanceTo(e: Entity, p: Pt): number {
  const segs = segments(e);
  let d = Infinity;
  for (const [a, b] of segs) {
    const f = foot(p, a, b);
    const on = f && dist(a, f) + dist(f, b) <= dist(a, b) * (1 + 1e-9);
    d = Math.min(d, on && f ? dist(p, f) : Math.min(dist(p, a), dist(p, b)));
  }
  if (e.kind === "arc") d = Math.min(d, Math.abs(dist(p, e.center) - e.radius));
  for (const q of endpoints(e)) d = Math.min(d, dist(p, q));
  if (e.kind === "north" || e.kind === "scalebar" || e.kind === "legend")
    d = Math.min(d, dist(p, e.at));
  return d;
}

export function DiagramEditor({
  initial,
  onNotice,
  onSaved,
}: {
  initial: DiagramRevision;
  onNotice: (s: string) => void;
  /** A new revision was saved. */
  onSaved: (r: DiagramRevision) => void;
}) {
  const [name, setName] = useState(initial.name);
  const [hist, setHist] = useState(() => history(initial.document));
  const doc = hist.present;
  const [saved, setSaved] = useState<{ revision: number; sha: string }>({
    revision: initial.number,
    sha: initial.sha256,
  });
  const [dirty, setDirty] = useState(false);
  const [tool, setTool] = useState<Tool>("select");
  const [clicks, setClicks] = useState<Pt[]>([]);
  const [cursor, setCursor] = useState<Snap | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [symbol, setSymbol] = useState(SYMBOLS[0]?.id ?? "car");
  const [activeLayer, setActiveLayer] = useState(doc.layers[0]?.id ?? "base");
  const [snaps, setSnaps] = useState({
    endpoint: true,
    midpoint: true,
    perpendicular: true,
    grid: true,
  });
  const [pending, setPending] = useState<{ kind: "text" | "point" | "scalebar"; at: Pt } | null>(
    null,
  );
  const [form, setForm] = useState({ text: "", height: 3, x: "", y: "", label: "", length: 5 });
  const [printing, setPrinting] = useState<{
    scale: number;
    paper: "A4" | "A3";
    landscape: boolean;
  } | null>(null);
  const host = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>({ center: [0, 0], scale: 40, width: 800, height: 600 });
  const dragging = useRef<{ x: number; y: number; moved: boolean } | null>(null);

  // Keep the view sized to its box.
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() =>
      setView((v) => ({ ...v, width: el.clientWidth, height: el.clientHeight })),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const change = useCallback((next: Diagram) => {
    setHist((h) => commit(h, next));
    setDirty(true);
  }, []);

  // Autosave a revision once edits settle, and on leaving the editor.
  const latest = useRef({ dirty, doc, name });
  latest.current = { dirty, doc, name };
  const save = useCallback(() => {
    const { doc, name } = latest.current;
    return api
      .diagramSave(initial.diagram_id, name, doc)
      .then((r) => {
        setSaved({ revision: r.number, sha: r.sha256 });
        setDirty(false);
        onSaved(r);
      })
      .catch((e) => onNotice(String(e)));
  }, [initial.diagram_id, onNotice, onSaved]);
  useEffect(() => {
    if (!dirty) return;
    const t = setTimeout(() => void save(), AUTOSAVE_MS);
    return () => clearTimeout(t);
  }, [dirty, doc, name, save]);
  useEffect(
    () => () => {
      if (latest.current.dirty) void save();
    },
    [save],
  );

  const layerOf = useMemo(() => new Map(doc.layers.map((l) => [l.id, l])), [doc.layers]);
  const visible = useMemo(
    () => doc.entities.filter((e) => layerOf.get(e.layer)?.visible !== false),
    [doc.entities, layerOf],
  );
  const grid = gridStep(view.scale);
  const legend = useMemo(() => legendItems(doc), [doc]);

  const snapAt = (s: Pt): Snap =>
    snap(toWorld(view, s), visible, {
      tolerance: 10 / view.scale,
      grid: snaps.grid ? grid / 2 : 0,
      from: clicks.length ? clicks[clicks.length - 1] : null,
      endpoint: snaps.endpoint,
      midpoint: snaps.midpoint,
      perpendicular: snaps.perpendicular,
    });

  const add = (e: EntityInput) => {
    if (layerOf.get(activeLayer)?.locked) {
      onNotice("The active layer is locked.");
      return;
    }
    change({
      ...doc,
      entities: [...doc.entities, { ...e, id: newId(), layer: activeLayer } as Entity],
    });
  };

  const finishPolyline = () => {
    if (clicks.length >= 2) add({ kind: "polyline", points: clicks, closed: false });
    setClicks([]);
  };

  const click = (p: Pt) => {
    const pts = [...clicks, p];
    switch (tool) {
      case "select": {
        const hit = visible
          .map((e) => [e, distanceTo(e, p)] as const)
          .filter(([, d]) => d <= 8 / view.scale)
          .sort((a, b) => a[1] - b[1])[0];
        setSelected(hit ? hit[0].id : null);
        return;
      }
      case "line":
        if (pts.length === 2) {
          add({ kind: "line", a: pts[0], b: pts[1] });
          setClicks([pts[1]]); // chain from the end
        } else setClicks(pts);
        return;
      case "polyline":
        setClicks(pts);
        return;
      case "arc":
        if (pts.length === 3) {
          const [c, s, e] = pts;
          add({
            kind: "arc",
            center: c,
            radius: dist(c, s),
            start: Math.atan2(s[1] - c[1], s[0] - c[0]),
            end: Math.atan2(e[1] - c[1], e[0] - c[0]),
          });
          setClicks([]);
        } else setClicks(pts);
        return;
      case "dimension":
        if (pts.length === 3) {
          const [a, b, o] = pts;
          const len = dist(a, b);
          // Signed distance of the third click to the left of a→b.
          const offset =
            len > 0 ? ((b[0] - a[0]) * (o[1] - a[1]) - (b[1] - a[1]) * (o[0] - a[0])) / len : 0;
          add({ kind: "dimension", a, b, offset });
          setClicks([]);
        } else setClicks(pts);
        return;
      case "north":
        add({ kind: "north", at: p, rotation: 0 });
        return;
      case "symbol":
        add({ kind: "symbol", symbol, at: p, rotation: 0, scale: 1 });
        return;
      case "marker":
        add({ kind: "marker", number: nextMarker(doc), at: p, note: "" });
        return;
      case "legend":
        add({ kind: "legend", at: p });
        return;
      case "measure":
        return;
      case "text":
      case "point":
      case "scalebar":
        setPending({ kind: tool, at: p });
        setForm((f) => ({ ...f, x: p[0].toFixed(3), y: p[1].toFixed(3) }));
        return;
    }
  };

  // Keyboard: Esc, Enter, Delete, undo/redo.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if ((ev.target as HTMLElement)?.closest("input, textarea, select")) return;
      if (ev.key === "Escape") {
        setClicks([]);
        setPending(null);
      } else if (ev.key === "Enter" && tool === "polyline") finishPolyline();
      else if ((ev.key === "Delete" || ev.key === "Backspace") && selected) {
        const e = doc.entities.find((x) => x.id === selected);
        if (e && !layerOf.get(e.layer)?.locked) {
          change({ ...doc, entities: doc.entities.filter((x) => x.id !== selected) });
          setSelected(null);
        }
      } else if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "z") {
        setHist((h) => (ev.shiftKey ? redo(h) : undo(h)));
        setDirty(true);
      } else if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "y") {
        setHist(redo);
        setDirty(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const screen = (ev: React.PointerEvent | React.WheelEvent): Pt => {
    const r = host.current!.getBoundingClientRect();
    return [ev.clientX - r.left, ev.clientY - r.top];
  };

  const layerUpdate = (id: string, patch: Partial<Layer>) =>
    change({ ...doc, layers: doc.layers.map((l) => (l.id === id ? { ...l, ...patch } : l)) });

  const submitPending = () => {
    if (!pending) return;
    if (pending.kind === "text" && form.text.trim())
      add({
        kind: "text",
        at: pending.at,
        text: form.text.trim(),
        height: form.height,
        rotation: 0,
      });
    if (pending.kind === "point") {
      const x = Number(form.x);
      const y = Number(form.y);
      if (!Number.isFinite(x) || !Number.isFinite(y)) {
        onNotice("Enter the point's coordinates in metres.");
        return;
      }
      add({ kind: "point", at: [x, y], label: form.label.trim(), measurement: null, sigma: null });
    }
    if (pending.kind === "scalebar" && form.length > 0)
      add({ kind: "scalebar", at: pending.at, length: form.length });
    setPending(null);
  };

  // --- rendering -------------------------------------------------------------------------
  const S = (p: Pt) => toScreen(view, p);
  const line = (a: Pt, b: Pt, props: React.SVGProps<SVGLineElement>) => {
    const [x1, y1] = S(a);
    const [x2, y2] = S(b);
    return <line x1={x1} y1={y1} x2={x2} y2={y2} {...props} />;
  };

  const drawEntity = (e: Entity) => {
    const color = layerOf.get(e.layer)?.color ?? "#e6e8eb";
    const sel = e.id === selected;
    const stroke = { stroke: sel ? "#f2a53a" : color, strokeWidth: sel ? 2.5 : 1.5, fill: "none" };
    switch (e.kind) {
      case "line":
        return line(e.a, e.b, { ...stroke, key: e.id });
      case "polyline": {
        const pts = [...e.points, ...(e.closed ? [e.points[0]] : [])]
          .map((p) => S(p).join(","))
          .join(" ");
        return <polyline key={e.id} points={pts} {...stroke} />;
      }
      case "arc": {
        const [sx, sy] = S([
          e.center[0] + e.radius * Math.cos(e.start),
          e.center[1] + e.radius * Math.sin(e.start),
        ]);
        const [ex, ey] = S([
          e.center[0] + e.radius * Math.cos(e.end),
          e.center[1] + e.radius * Math.sin(e.end),
        ]);
        let sweep = e.end - e.start;
        while (sweep < 0) sweep += 2 * Math.PI;
        const r = e.radius * view.scale;
        // Screen y is flipped, so anticlockwise in the world is sweep-flag 0 on screen.
        return (
          <path
            key={e.id}
            d={`M ${sx} ${sy} A ${r} ${r} 0 ${sweep > Math.PI ? 1 : 0} 0 ${ex} ${ey}`}
            {...stroke}
          />
        );
      }
      case "dimension": {
        const len = dist(e.a, e.b);
        if (len === 0) return null;
        const n: Pt = [(-(e.b[1] - e.a[1]) / len) * e.offset, ((e.b[0] - e.a[0]) / len) * e.offset];
        const a2: Pt = [e.a[0] + n[0], e.a[1] + n[1]];
        const b2: Pt = [e.b[0] + n[0], e.b[1] + n[1]];
        const [mx, my] = S([(a2[0] + b2[0]) / 2, (a2[1] + b2[1]) / 2]);
        return (
          <g key={e.id}>
            {line(e.a, a2, { ...stroke, strokeWidth: 0.8 })}
            {line(e.b, b2, { ...stroke, strokeWidth: 0.8 })}
            {line(a2, b2, { ...stroke, markerStart: "url(#tick)", markerEnd: "url(#tick)" })}
            <text x={mx} y={my - 4} className="dg-text" textAnchor="middle" fill={stroke.stroke}>
              {fmt(len)}
            </text>
          </g>
        );
      }
      case "text": {
        const [x, y] = S(e.at);
        return (
          <text
            key={e.id}
            x={x}
            y={y}
            className="dg-text"
            fill={stroke.stroke}
            fontSize={Math.max(10, e.height * 4)}
          >
            {e.text}
          </text>
        );
      }
      case "point": {
        const [x, y] = S(e.at);
        return (
          <g key={e.id}>
            <circle cx={x} cy={y} r={4} stroke={stroke.stroke} strokeWidth={1.5} fill="none" />
            <line x1={x - 6} y1={y} x2={x + 6} y2={y} stroke={stroke.stroke} />
            <line x1={x} y1={y - 6} x2={x} y2={y + 6} stroke={stroke.stroke} />
            {e.label && (
              <text x={x + 8} y={y - 6} className="dg-text" fill={stroke.stroke}>
                {e.label}
              </text>
            )}
          </g>
        );
      }
      case "symbol": {
        const def = symbolById.get(e.symbol);
        if (!def) return null;
        const [x, y] = S(e.at);
        // The symbol's 100-unit box spans def.size × scale metres on the ground.
        const k = (def.size * e.scale * view.scale) / 100;
        return (
          <g
            key={e.id}
            transform={`translate(${x} ${y}) rotate(${(-e.rotation * 180) / Math.PI}) scale(${k})`}
            color={stroke.stroke}
            dangerouslySetInnerHTML={{ __html: def.body }}
          />
        );
      }
      case "marker": {
        const [x, y] = S(e.at);
        return (
          <g key={e.id}>
            <path
              d={`M ${x - 11} ${y + 9} L ${x} ${y - 13} L ${x + 11} ${y + 9} Z`}
              fill="#f2c94c"
              stroke={sel ? "#f2a53a" : "#1e1f22"}
            />
            <text x={x} y={y + 6} className="dg-marker" textAnchor="middle">
              {e.number}
            </text>
          </g>
        );
      }
      case "legend": {
        const [x, y] = S(e.at);
        return <LegendBox key={e.id} x={x} y={y} items={legend} color={stroke.stroke} />;
      }
      case "north": {
        const [x, y] = S(e.at);
        return (
          <g key={e.id} transform={`translate(${x} ${y}) rotate(${(-e.rotation * 180) / Math.PI})`}>
            <path d="M 0 -26 L 9 8 L 0 2 L -9 8 Z" fill={stroke.stroke} />
            <text y={24} className="dg-text" textAnchor="middle" fill={stroke.stroke}>
              N
            </text>
          </g>
        );
      }
      case "scalebar": {
        const [x0, y0] = S(e.at);
        const [x1] = S([e.at[0] + e.length, e.at[1]]);
        return (
          <g key={e.id}>
            <rect x={x0} y={y0 - 4} width={(x1 - x0) / 2} height={4} fill={stroke.stroke} />
            <rect x={x0} y={y0 - 4} width={x1 - x0} height={4} fill="none" stroke={stroke.stroke} />
            <text x={x0} y={y0 + 12} className="dg-text" fill={stroke.stroke}>
              0
            </text>
            <text x={x1} y={y0 + 12} className="dg-text" textAnchor="end" fill={stroke.stroke}>
              {e.length} m
            </text>
          </g>
        );
      }
      default:
        return null;
    }
  };

  // Grid lines across the view.
  const gridLines = [];
  {
    const tl = toWorld(view, [0, 0]);
    const br = toWorld(view, [view.width, view.height]);
    for (let x = Math.ceil(tl[0] / grid) * grid; x <= br[0]; x += grid)
      gridLines.push(line([x, tl[1]], [x, br[1]], { key: `gx${x}`, className: "dg-grid" }));
    for (let y = Math.ceil(br[1] / grid) * grid; y <= tl[1]; y += grid)
      gridLines.push(line([tl[0], y], [br[0], y], { key: `gy${y}`, className: "dg-grid" }));
  }

  const preview =
    cursor && clicks.length > 0 && tool !== "select"
      ? line(clicks[clicks.length - 1], cursor.point, { className: "dg-preview" })
      : null;

  return (
    <div className="dg">
      <div className="dg-toolbar">
        <input
          className="dg-name"
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            setDirty(true);
          }}
          aria-label="Diagram name"
        />
        {TOOLS.map(([t, label]) => (
          <button
            key={t}
            className={tool === t ? "primary" : ""}
            onClick={() => {
              setTool(t);
              setClicks([]);
            }}
          >
            {label}
          </button>
        ))}
        <span className="dg-sep" />
        {(["endpoint", "midpoint", "perpendicular", "grid"] as const).map((k) => (
          <label key={k} className="inline">
            <input
              type="checkbox"
              checked={snaps[k]}
              onChange={(e) => setSnaps({ ...snaps, [k]: e.target.checked })}
            />
            {k}
          </label>
        ))}
        <button onClick={() => setPrinting({ scale: 100, paper: "A4", landscape: true })}>
          Print to scale…
        </button>
        <span className="muted dg-status">
          {dirty ? "Saving…" : `Revision ${saved.revision} saved`}
        </span>
      </div>
      <div className="dg-body">
        <div
          ref={host}
          className="dg-canvas"
          onWheel={(ev) => setView((v) => zoomAt(v, screen(ev), ev.deltaY < 0 ? 1.15 : 1 / 1.15))}
          onPointerDown={(ev) => {
            dragging.current = { x: ev.clientX, y: ev.clientY, moved: false };
            (ev.target as Element).setPointerCapture?.(ev.pointerId);
          }}
          onPointerMove={(ev) => {
            const d = dragging.current;
            if (d && (ev.buttons & 1 || ev.buttons & 4) && (tool === "select" || ev.buttons & 4)) {
              const dx = ev.clientX - d.x;
              const dy = ev.clientY - d.y;
              if (Math.abs(dx) + Math.abs(dy) > 2) {
                d.moved = true;
                d.x = ev.clientX;
                d.y = ev.clientY;
                setView((v) => pan(v, dx, dy));
              }
            }
            setCursor(
              tool === "select"
                ? { point: toWorld(view, screen(ev)), kind: "none" }
                : snapAt(screen(ev)),
            );
          }}
          onPointerUp={(ev) => {
            const d = dragging.current;
            dragging.current = null;
            if (d?.moved || ev.button !== 0) return;
            click(tool === "select" ? toWorld(view, screen(ev)) : snapAt(screen(ev)).point);
          }}
          onDoubleClick={() => tool === "polyline" && finishPolyline()}
          onPointerLeave={() => setCursor(null)}
        >
          <svg width={view.width} height={view.height} role="img" aria-label="Diagram canvas">
            <defs>
              <marker
                id="tick"
                viewBox="-5 -5 10 10"
                markerWidth="10"
                markerHeight="10"
                orient="auto"
              >
                <line x1="-4" y1="4" x2="4" y2="-4" stroke="currentColor" />
              </marker>
            </defs>
            {gridLines}
            {visible.map(drawEntity)}
            {preview}
            {tool === "polyline" && clicks.length > 1 && (
              <polyline
                points={clicks.map((p) => S(p).join(",")).join(" ")}
                className="dg-preview"
              />
            )}
            {cursor && cursor.kind !== "none" && (
              <rect
                x={S(cursor.point)[0] - 5}
                y={S(cursor.point)[1] - 5}
                width={10}
                height={10}
                className={`dg-snap dg-snap-${cursor.kind}`}
              />
            )}
          </svg>
          <div className="dg-hint">
            {STEPS[tool][Math.min(clicks.length, STEPS[tool].length - 1)]}
            {cursor && (
              <span className="dg-coords">
                x {cursor.point[0].toFixed(3)} y {cursor.point[1].toFixed(3)} m
                {cursor.kind !== "none" ? ` · ${cursor.kind}` : ""}
              </span>
            )}
          </div>
          {printing && (
            <div
              className="dg-form"
              onPointerDown={(e) => e.stopPropagation()}
              onPointerUp={(e) => e.stopPropagation()}
            >
              <strong>Print to scale</strong>
              <label>
                Scale
                <select
                  value={printing.scale}
                  onChange={(e) => setPrinting({ ...printing, scale: Number(e.target.value) })}
                >
                  {[10, 20, 25, 50, 100, 200, 250, 500, 1000].map((n) => (
                    <option key={n} value={n}>
                      1:{n}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Paper
                <select
                  value={printing.paper}
                  onChange={(e) =>
                    setPrinting({ ...printing, paper: e.target.value as "A4" | "A3" })
                  }
                >
                  <option value="A4">A4</option>
                  <option value="A3">A3</option>
                </select>
              </label>
              <label className="inline">
                <input
                  type="checkbox"
                  checked={printing.landscape}
                  onChange={(e) => setPrinting({ ...printing, landscape: e.target.checked })}
                />
                Landscape
              </label>
              <p className="muted">
                Prints the saved revision. Print the PDF at actual size (100 %) for the scale to
                hold on paper.
              </p>
              <div className="buttons">
                <button onClick={() => setPrinting(null)}>Cancel</button>
                <button
                  className="primary"
                  onClick={async () => {
                    const path = await saveDialog({
                      title: "Save diagram PDF",
                      defaultPath: `${name}-1_${printing.scale}.pdf`,
                      filters: [{ name: "PDF", extensions: ["pdf"] }],
                    });
                    if (!path) return;
                    try {
                      if (latest.current.dirty) await save();
                      const sha = await api.diagramPdf(
                        initial.diagram_id,
                        printing.scale,
                        printing.paper,
                        printing.landscape,
                        path,
                      );
                      onNotice(
                        `Diagram saved to ${path} at 1:${printing.scale} (SHA-256 ${sha}; recorded in the audit log).`,
                      );
                      setPrinting(null);
                    } catch (e) {
                      onNotice(String(e));
                    }
                  }}
                >
                  Save PDF…
                </button>
              </div>
            </div>
          )}
          {tool === "measure" && (
            <MeasureDialog
              points={visible.filter(
                (e): e is Extract<Entity, { kind: "point" }> => e.kind === "point",
              )}
              onPlace={add}
              onClose={() => setTool("select")}
            />
          )}
          {pending && (
            <div
              className="dg-form"
              onPointerDown={(e) => e.stopPropagation()}
              onPointerUp={(e) => e.stopPropagation()}
            >
              {pending.kind === "text" && (
                <>
                  <label>
                    Text
                    <input
                      autoFocus
                      value={form.text}
                      onChange={(e) => setForm({ ...form, text: e.target.value })}
                    />
                  </label>
                  <label>
                    Height on paper (mm)
                    <input
                      type="number"
                      value={form.height}
                      min={1}
                      onChange={(e) => setForm({ ...form, height: +e.target.value })}
                    />
                  </label>
                </>
              )}
              {pending.kind === "point" && (
                <>
                  <label>
                    x (m)
                    <input
                      autoFocus
                      value={form.x}
                      onChange={(e) => setForm({ ...form, x: e.target.value })}
                    />
                  </label>
                  <label>
                    y (m)
                    <input
                      value={form.y}
                      onChange={(e) => setForm({ ...form, y: e.target.value })}
                    />
                  </label>
                  <label>
                    Label
                    <input
                      value={form.label}
                      onChange={(e) => setForm({ ...form, label: e.target.value })}
                    />
                  </label>
                </>
              )}
              {pending.kind === "scalebar" && (
                <label>
                  Length (m)
                  <input
                    autoFocus
                    type="number"
                    value={form.length}
                    min={0.1}
                    onChange={(e) => setForm({ ...form, length: +e.target.value })}
                  />
                </label>
              )}
              <div className="buttons">
                <button onClick={() => setPending(null)}>Cancel</button>
                <button className="primary" onClick={submitPending}>
                  Place
                </button>
              </div>
            </div>
          )}
        </div>
        <aside className="dg-side">
          {tool === "symbol" && (
            <>
              <h3>Symbols</h3>
              <div className="dg-palette">
                {SYMBOLS.map((d) => (
                  <button
                    key={d.id}
                    className={d.id === symbol ? "primary" : ""}
                    title={`${d.name} (${d.size} m)`}
                    onClick={() => setSymbol(d.id)}
                  >
                    <svg
                      viewBox="-50 -50 100 100"
                      width={28}
                      height={28}
                      color="currentColor"
                      dangerouslySetInnerHTML={{ __html: d.body }}
                    />
                  </button>
                ))}
              </div>
            </>
          )}
          {doc.entities.some((e) => e.kind === "marker") && (
            <button onClick={() => change(renumberMarkers(doc))}>
              Renumber markers (reading order)
            </button>
          )}
          <h3>Layers</h3>
          {doc.layers.map((l) => (
            <div key={l.id} className={`dg-layer${l.id === activeLayer ? " active" : ""}`}>
              <input
                type="radio"
                checked={l.id === activeLayer}
                onChange={() => setActiveLayer(l.id)}
                aria-label={`Draw on ${l.name}`}
              />
              <input
                type="color"
                value={l.color}
                onChange={(e) => layerUpdate(l.id, { color: e.target.value })}
                aria-label="Layer colour"
              />
              <input
                className="dg-layer-name"
                value={l.name}
                onChange={(e) => layerUpdate(l.id, { name: e.target.value })}
              />
              <label className="inline" title="Visible">
                <input
                  type="checkbox"
                  checked={l.visible}
                  onChange={(e) => layerUpdate(l.id, { visible: e.target.checked })}
                />
                show
              </label>
              <label className="inline" title="Locked">
                <input
                  type="checkbox"
                  checked={l.locked}
                  onChange={(e) => layerUpdate(l.id, { locked: e.target.checked })}
                />
                lock
              </label>
            </div>
          ))}
          <button
            onClick={() => {
              const id = newId();
              change({
                ...doc,
                layers: [
                  ...doc.layers,
                  {
                    id,
                    name: `Layer ${doc.layers.length + 1}`,
                    visible: true,
                    locked: false,
                    color: "#8ab4f8",
                  },
                ],
              });
              setActiveLayer(id);
            }}
          >
            Add layer
          </button>
          <p className="muted">
            {doc.entities.length} item(s). Every save is a new revision in the project, logged with
            its hash.
          </p>
          <p className="muted dg-hash">SHA-256 {saved.sha.slice(0, 16)}…</p>
        </aside>
      </div>
    </div>
  );
}

/** The legend, drawn in screen space from `legendItems` (so it follows every change). */
function LegendBox({
  x,
  y,
  items,
  color,
}: {
  x: number;
  y: number;
  items: LegendItem[];
  color: string;
}) {
  const row = 22;
  const width = 220;
  const height = 28 + Math.max(items.length, 1) * row;
  return (
    <g transform={`translate(${x} ${y})`} className="dg-legend">
      <rect width={width} height={height} fill="#1e1f22" stroke={color} />
      <text x={10} y={18} className="dg-text" fill={color} fontWeight="bold">
        Legend
      </text>
      {items.length === 0 && (
        <text x={10} y={42} className="dg-text" fill={color}>
          (nothing to list yet)
        </text>
      )}
      {items.map((it, i) => {
        const cy = 28 + i * row + row / 2;
        let glyph;
        if (typeof it.glyph === "object") {
          const def = symbolById.get(it.glyph.symbol);
          glyph = def ? (
            <g
              transform={`translate(18 ${cy}) scale(0.16)`}
              color={color}
              dangerouslySetInnerHTML={{ __html: def.body }}
            />
          ) : null;
        } else if (it.glyph === "marker") {
          glyph = <path d={`M 10 ${cy + 7} L 18 ${cy - 9} L 26 ${cy + 7} Z`} fill="#f2c94c" />;
        } else {
          glyph = (
            <g>
              <circle cx={18} cy={cy} r={4} stroke={color} fill="none" />
              {it.glyph === "measured" && (
                <circle cx={18} cy={cy} r={8} stroke={color} fill="none" strokeDasharray="2 2" />
              )}
            </g>
          );
        }
        return (
          <g key={it.key}>
            {glyph}
            <text x={36} y={cy + 4} className="dg-text" fill={color}>
              {it.label}
              {typeof it.glyph === "object" && it.count > 1 ? ` (${it.count})` : ""}
            </text>
          </g>
        );
      })}
    </g>
  );
}
