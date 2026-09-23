import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, type DiagramRevision, type ProjectInfo, type Resolved, type StateView } from "../api";
import { formatCount } from "../format";
import { Engine, type ClipMode, type Stats } from "./engine";
import { TOOL_POINTS, type MeasurementRecord } from "./measureFormat";
import type { ColorMode, PickHit, SceneData } from "./pointcloud";
import { ScenePanel, type BuildStatus } from "./ScenePanel";
import { SceneBuilder } from "../scene3d/SceneBuilder";
import { TrajectoryPanel } from "../tools/trajectory/TrajectoryPanel";

export type Tool = "orbit" | MeasurementRecord["kind"] | "lasso";

export interface ViewSettings {
  color: ColorMode;
  pointSize: number;
  edl: boolean;
  edlStrength: number;
  clip: ClipMode;
  gizmo: "translate" | "scale";
  plane: { on: boolean; axis: 0 | 1 | 2; offset: number; flip: boolean };
}

const DEFAULT_SETTINGS: ViewSettings = {
  color: "rgb",
  pointSize: 1,
  edl: true,
  edlStrength: 1,
  clip: "off",
  gizmo: "translate",
  plane: { on: false, axis: 2, offset: 0, flip: false },
};

export function Viewport({
  project,
  diagrams,
  onNotice,
}: {
  project: ProjectInfo | null;
  /** The project's diagrams at their newest revisions (for extruding). */
  diagrams: DiagramRevision[];
  onNotice: (message: string | null) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<Engine | null>(null);
  const [scene, setScene] = useState<SceneData | null>(null);
  const [state, setState] = useState<StateView | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [builds, setBuilds] = useState<Record<string, BuildStatus>>({});
  const [tool, setTool] = useState<Tool>("orbit");
  const [picks, setPicks] = useState<{ hit: PickHit; at: Resolved }[]>([]);
  const [planeDone, setPlaneDone] = useState(false);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [busy, setBusy] = useState<string | null>(null);
  const [lasso, setLasso] = useState<[number, number][]>([]);
  /** The scene builder waiting for a click on the cloud (snapping, picking a base). */
  const [pickRequest, setPickRequest] = useState<{
    hint: string;
    then: (hit: PickHit) => void;
  } | null>(null);
  const [lassoConfirm, setLassoConfirm] = useState<{
    polygon: [number, number][];
    visible: number;
    all: number;
    mode: "visible_surface" | "all_depths";
  } | null>(null);

  // One engine for the lifetime of the component.
  useEffect(() => {
    let last = 0;
    const engine = new Engine(hostRef.current!, (s) => {
      const now = performance.now();
      // Throttle, but never drop the last frame before the view goes idle.
      if (now - last > 250 || !s.loading) {
        last = now;
        setStats(s);
      }
    });
    engineRef.current = engine;
    return () => engine.dispose();
  }, []);

  const loadScene = useCallback(
    () =>
      Promise.all([api.sceneView(), api.analysisState()]).then(([s, st]) => {
        setScene(s);
        setState(st);
      }),
    [],
  );

  const root = project?.root;
  useEffect(() => {
    if (!root) return;
    Promise.all([api.sceneView(), api.analysisState()])
      .then(([s, st]) => {
        setScene(s);
        setState(st);
      })
      .catch((e) => onNotice(String(e)));
  }, [root, onNotice]);
  // With no project open nothing is shown, whatever was loaded before.
  const shownScene = project ? scene : null;

  useEffect(() => {
    const subs = [
      listen("scene-changed", () => void loadScene().catch((e) => onNotice(String(e)))),
      listen<{
        scan: string;
        name: string;
        progress: BuildStatus["progress"];
        error: string | null;
      }>("octree-progress", ({ payload }) =>
        setBuilds((b) => ({
          ...b,
          [payload.scan]: {
            name: payload.name,
            progress: payload.progress,
            error: payload.error,
            done: !payload.progress,
          },
        })),
      ),
      listen<[number, number]>("cleanup-progress", ({ payload }) =>
        setBusy(`Working… ${Math.round((payload[0] / Math.max(payload[1], 1)) * 100)}%`),
      ),
    ];
    return () => subs.forEach((s) => void s.then((f) => f()));
  }, [loadScene, onNotice]);

  useEffect(() => engineRef.current?.setScene(shownScene), [shownScene]);
  useEffect(() => {
    if (state && shownScene) engineRef.current?.setState(state);
  }, [state, shownScene]);

  // View settings → engine.
  useEffect(() => {
    const e = engineRef.current;
    if (!e) return;
    e.setColorMode(settings.color);
    e.setPointSize(settings.pointSize);
    e.setEdl(settings.edl, settings.edlStrength);
    e.setClipBox(settings.clip, settings.gizmo);
    e.setClipPlane(
      settings.plane.on,
      settings.plane.axis,
      settings.plane.offset,
      settings.plane.flip,
    );
  }, [settings, scene]);

  // Measurement tools refine the cursor's surroundings to full resolution (item 8).
  useEffect(() => {
    const e = engineRef.current;
    if (!e) return;
    e.focusActive = tool !== "orbit" && tool !== "lasso";
    e.controls.enabled = tool !== "lasso";
    e.requestRender();
  }, [tool]);

  useEffect(() => engineRef.current?.setMarkers(picks.map((p) => p.at.project)), [picks]);

  const getEngine = useCallback(() => engineRef.current, []);

  const resetTool = useCallback(() => {
    setPicks([]);
    setPlaneDone(false);
    setLasso([]);
  }, []);

  const finish = useCallback(
    async (kind: MeasurementRecord["kind"], all: { hit: PickHit }[]) => {
      try {
        setState(
          await api.measure(
            kind,
            all.map((p) => p.hit),
          ),
        );
        onNotice(null);
      } catch (e) {
        onNotice(String(e));
      }
      resetTool();
    },
    [onNotice, resetTool],
  );

  const onClick = useCallback(
    async (x: number, y: number) => {
      const e = engineRef.current;
      if (e && pickRequest) {
        const hit = e.pick(x, y);
        if (!hit || "refused" in hit) {
          onNotice(hit ? hit.refused : "No point under the cursor. Zoom in, or move closer.");
          return;
        }
        setPickRequest(null);
        pickRequest.then(hit);
        return;
      }
      if (!e || tool === "orbit" || tool === "lasso") return;
      const hit = e.pick(x, y);
      if (!hit) {
        onNotice("No point under the cursor. Zoom in, or move closer to the surface.");
        return;
      }
      if ("refused" in hit) {
        onNotice(hit.refused);
        return;
      }
      let at: Resolved;
      try {
        at = await api.pickResolve(hit);
      } catch (err) {
        onNotice(String(err));
        return;
      }
      const next = [...picks, { hit, at }];
      setPicks(next);
      const need = TOOL_POINTS[tool];
      if (need.exact && next.length === need.exact) void finish(tool, next);
      if (tool === "height" && planeDone) void finish(tool, next);
    },
    [tool, picks, planeDone, finish, onNotice, pickRequest],
  );

  // Keyboard: Enter completes open-ended tools, Escape cancels, Ctrl+Z / Ctrl+Y undo cleanup.
  useEffect(() => {
    const onKey = async (ev: KeyboardEvent) => {
      if (ev.target instanceof HTMLInputElement) return;
      if (ev.key === "Escape") {
        resetTool();
        setPickRequest(null);
      }
      if (ev.key === "Enter" && tool === "area" && picks.length >= 3) void finish("area", picks);
      if (ev.key === "Enter" && tool === "height" && picks.length >= 3 && !planeDone)
        setPlaneDone(true);
      if (ev.key === "t") setSettings((s) => ({ ...s, gizmo: "translate" }));
      if (ev.key === "s" && !ev.ctrlKey) setSettings((s) => ({ ...s, gizmo: "scale" }));
      if (ev.ctrlKey && (ev.key === "z" || ev.key === "y") && state) {
        const ops = state.cleanups;
        const target =
          ev.key === "z"
            ? [...ops].reverse().find((o) => o.active)
            : [...ops].reverse().find((o) => !o.active);
        if (target) {
          ev.preventDefault();
          try {
            setState(await api.cleanupSetActive(target.id, ev.key === "y"));
          } catch (e) {
            onNotice(String(e));
          }
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tool, picks, planeDone, state, finish, resetTool, onNotice]);

  // Pointer handling on the canvas: click to pick (not after a drag), drag to lasso.
  const down = useRef<{ x: number; y: number } | null>(null);
  const local = (ev: React.PointerEvent) => {
    const r = hostRef.current!.getBoundingClientRect();
    return { x: ev.clientX - r.left, y: ev.clientY - r.top };
  };
  const onPointerDown = (ev: React.PointerEvent) => {
    down.current = local(ev);
    if (tool === "lasso") setLasso([[down.current.x, down.current.y]]);
  };
  const onPointerMove = (ev: React.PointerEvent) => {
    if (tool === "lasso" && down.current && ev.buttons & 1) {
      const p = local(ev);
      setLasso((l) => [...l, [p.x, p.y]]);
    }
  };
  const onPointerUp = async (ev: React.PointerEvent) => {
    const start = down.current;
    down.current = null;
    if (!start) return;
    const p = local(ev);
    if (tool === "lasso") {
      const e = engineRef.current;
      if (lasso.length >= 3 && e) {
        // Count both modes first, so the examiner sees what each would remove.
        setBusy("Counting points…");
        try {
          const [visible, all] = await Promise.all([
            api.cleanupPreview(e.lassoRequest(lasso, "visible_surface")),
            api.cleanupPreview(e.lassoRequest(lasso, "all_depths")),
          ]);
          setLassoConfirm({ polygon: lasso, visible, all, mode: "visible_surface" });
        } catch (err) {
          onNotice(String(err));
        } finally {
          setBusy(null);
        }
      }
      setLasso([]);
      return;
    }
    if (Math.hypot(p.x - start.x, p.y - start.y) < 4) void onClick(p.x, p.y);
  };

  const runCleanup = async (request: Parameters<typeof api.cleanupApply>[0]) => {
    setBusy("Working…");
    try {
      setState(await api.cleanupApply(request));
      onNotice(null);
    } catch (e) {
      onNotice(String(e));
    } finally {
      setBusy(null);
    }
  };

  const hint = pickRequest
    ? `${pickRequest.hint} Esc cancels.`
    : tool === "orbit"
      ? null
      : tool === "lasso"
        ? "Drag around the points to delete. Undo with Ctrl+Z."
        : tool === "height" && planeDone
          ? "Now click the point to measure."
          : `${TOOL_POINTS[tool].hint} ${picks.length ? `(${picks.length} picked)` : ""} Esc cancels.`;

  return (
    <div className="viewer">
      <div
        ref={hostRef}
        className={`viewport tool-${tool}`}
        data-testid="viewport"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
      >
        {lasso.length > 1 && (
          <svg className="lasso">
            <polyline points={lasso.map((p) => p.join(",")).join(" ")} />
          </svg>
        )}
        {hint && <div className="tool-hint">{hint}</div>}
        {busy && <div className="tool-hint busy">{busy}</div>}
        {stats && shownScene && shownScene.scans.length > 0 && (
          <div className="stats">
            {(stats.drawn / 1e6).toFixed(1)}M of {(stats.budget / 1e6).toFixed(1)}M points ·{" "}
            {stats.fps.toFixed(0)} fps · node selection {stats.selectionMs.toFixed(1)} ms
            {stats.loading ? " · loading" : ""}
          </div>
        )}
      </div>
      {lassoConfirm && (
        <div className="overlay">
          <div className="dialog" role="dialog" aria-label="Lasso delete">
            <h2>Delete points inside the lasso?</h2>
            <label className="inline">
              <input
                type="radio"
                checked={lassoConfirm.mode === "visible_surface"}
                onChange={() => setLassoConfirm({ ...lassoConfirm, mode: "visible_surface" })}
              />
              Visible surface only: {formatCount(lassoConfirm.visible, "point")}
            </label>
            <label className="inline">
              <input
                type="radio"
                checked={lassoConfirm.mode === "all_depths"}
                onChange={() => setLassoConfirm({ ...lassoConfirm, mode: "all_depths" })}
              />
              All depths, including points hidden behind: {formatCount(lassoConfirm.all, "point")}
            </label>
            <p className="muted">
              Evidence is not changed. The operation is logged and can be undone.
            </p>
            <div className="buttons">
              <button onClick={() => setLassoConfirm(null)}>Cancel</button>
              <button
                className="primary"
                onClick={() => {
                  const e = engineRef.current;
                  const c = lassoConfirm;
                  setLassoConfirm(null);
                  if (e) void runCleanup(e.lassoRequest(c.polygon, c.mode));
                }}
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
      {project && (
        <ScenePanel
          scene={shownScene}
          state={state}
          builds={builds}
          tool={tool}
          setTool={(t) => {
            resetTool();
            setTool(t);
          }}
          settings={settings}
          setSettings={setSettings}
          region={() => engineRef.current?.clipRegion() ?? null}
          runCleanup={runCleanup}
          setState={setState}
          onNotice={onNotice}
        >
          <SceneBuilder
            key={project.root}
            engine={getEngine}
            diagramList={diagrams}
            origin={shownScene?.origin.join() ?? ""}
            requestPick={(hint, then) => {
              setTool("orbit");
              setPickRequest({ hint, then });
            }}
            onNotice={onNotice}
          />
          <TrajectoryPanel
            key={`t${project.root}`}
            engine={getEngine}
            origin={shownScene?.origin.join() ?? ""}
            requestPick={(hint, then) => {
              setTool("orbit");
              setPickRequest({ hint, then });
            }}
            onNotice={onNotice}
          />
        </ScenePanel>
      )}
    </div>
  );
}
