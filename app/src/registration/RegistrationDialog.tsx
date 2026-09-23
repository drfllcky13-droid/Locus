import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";
import { api, type RegistrationParams, type RegistrationRecord } from "../api";
import { layout, type EdgeStyle } from "./graph";

const SIZE = 420;
const MARGIN = 40;
const STROKE: Record<EdgeStyle, string> = {
  ok: "#5cc98a",
  flagged: "#ff7b72",
  untested: "#9aa0a8",
  "shape-only": "#f2a53a",
  deleted: "#5a5e66",
};

const mm = (m: number | null | undefined) => (m == null ? "—" : `${(m * 1000).toFixed(2)} mm`);

/** Register scans: run, review the link graph, delete or force links and re-solve, apply. */
export function RegistrationDialog({
  onClose,
  onNotice,
}: {
  onClose: () => void;
  onNotice: (s: string) => void;
}) {
  const [regs, setRegs] = useState<RegistrationRecord[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [link, setLink] = useState<number | null>(null);
  const [del, setDel] = useState<Set<number>>(new Set());
  const [force, setForce] = useState<Set<number>>(new Set());
  // Form values in the UI's units (millimetres); converted to SI when run.
  const [form, setForm] = useState({
    spheres: true,
    sphereDiameterMm: 145,
    boards: true,
    boardMm: 300,
    cloud: true,
    useFilePoses: true,
    cloudSigmaMm: 2,
    toleranceMm: 5,
  });

  useEffect(() => {
    void api
      .registrations()
      .then((r) => {
        setRegs(r);
        setSelected(r.length ? r[r.length - 1].id : null);
      })
      .catch((e) => onNotice(String(e)));
    const unlisten = listen<string>("registration-progress", ({ payload }) => setBusy(payload));
    return () => {
      void unlisten.then((f) => f());
    };
  }, [onNotice]);

  const reg = regs.find((r) => r.id === selected) ?? null;
  const graph = useMemo(() => (reg ? layout(reg, SIZE, MARGIN, del) : null), [reg, del]);

  /** New list from the backend; `newest` selects the newest registration. */
  const update = (next: RegistrationRecord[], newest: boolean) => {
    setRegs(next);
    if (newest) setSelected(next.length ? next[next.length - 1].id : null);
    setLink(null);
    setDel(new Set());
    setForce(new Set());
  };

  const run = async () => {
    const params: RegistrationParams = {
      sphere_radius: form.spheres ? form.sphereDiameterMm / 2000 : null,
      board_size: form.boards ? form.boardMm / 1000 : null,
      cloud: form.cloud,
      use_file_poses: form.useFilePoses,
      cloud_sigma: form.cloudSigmaMm / 1000,
      target_tolerance: form.toleranceMm / 1000,
      max_points: 8_000_000,
      control: [],
    };
    setBusy("Starting…");
    try {
      update(await api.registrationRun(params), true);
    } catch (e) {
      onNotice(String(e));
    } finally {
      setBusy(null);
    }
  };

  const resolve = async () => {
    if (!reg) return;
    setBusy("Re-solving…");
    try {
      update(await api.registrationEdit(reg.id, [...del], [...force]), true);
    } catch (e) {
      onNotice(String(e));
    } finally {
      setBusy(null);
    }
  };

  const apply = async (id: number | null) => {
    setBusy(id === null ? "Reverting…" : "Applying…");
    try {
      update(await api.registrationApply(id), false);
    } catch (e) {
      onNotice(String(e));
    } finally {
      setBusy(null);
    }
  };

  const exportPdf = async (id: number) => {
    const path = await save({
      title: "Save registration report",
      defaultPath: `registration-${id}.pdf`,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!path) return;
    setBusy("Writing report…");
    try {
      const sha = await api.registrationReport(id, path);
      onNotice(`Report saved to ${path} (SHA-256 ${sha}; recorded in the audit log).`);
    } catch (e) {
      onNotice(String(e));
    } finally {
      setBusy(null);
    }
  };

  const toggle = (set: Set<number>, i: number, other: Set<number>) => {
    const next = new Set(set);
    if (next.has(i)) next.delete(i);
    else {
      next.add(i);
      other.delete(i);
    }
    return next;
  };

  const s = reg?.result.summary;
  const l = reg && link !== null ? reg.result.links[link] : null;
  const r = reg && link !== null ? reg.result.reports[link] : null;
  const overlap = reg && link !== null ? reg.result.extra.overlap?.[link] : null;
  const scanName = (i: number | null) =>
    i === null ? "survey control" : (reg?.result.scans[i]?.name ?? `scan ${i}`);
  const applied = regs.find((x) => x.applied);

  return (
    <div className="overlay">
      <div className="dialog wide" role="dialog" aria-label="Register scans">
        <h2>Register scans</h2>
        <div className="reg-form">
          <label className="inline">
            <input
              type="checkbox"
              checked={form.spheres}
              onChange={(e) => setForm({ ...form, spheres: e.target.checked })}
            />
            Spheres, diameter
            <input
              type="number"
              value={form.sphereDiameterMm}
              min={10}
              onChange={(e) => setForm({ ...form, sphereDiameterMm: +e.target.value })}
            />{" "}
            mm
          </label>
          <label className="inline">
            <input
              type="checkbox"
              checked={form.boards}
              onChange={(e) => setForm({ ...form, boards: e.target.checked })}
            />
            Checkerboards, edge
            <input
              type="number"
              value={form.boardMm}
              min={50}
              onChange={(e) => setForm({ ...form, boardMm: +e.target.value })}
            />{" "}
            mm
          </label>
          <label className="inline">
            <input
              type="checkbox"
              checked={form.cloud}
              onChange={(e) => setForm({ ...form, cloud: e.target.checked })}
            />
            Cloud-to-cloud, each link point ±
            <input
              type="number"
              value={form.cloudSigmaMm}
              min={0.1}
              step={0.1}
              onChange={(e) => setForm({ ...form, cloudSigmaMm: +e.target.value })}
            />{" "}
            mm
          </label>
          <label className="inline">
            <input
              type="checkbox"
              checked={form.useFilePoses}
              onChange={(e) => setForm({ ...form, useFilePoses: e.target.checked })}
            />
            Start from the poses in the files (scanner pre-registration)
          </label>
          <div className="buttons">
            <button className="primary" disabled={busy !== null} onClick={() => void run()}>
              Run registration
            </button>
          </div>
        </div>
        {busy && (
          <p className="muted" role="status">
            {busy}
          </p>
        )}

        {regs.length > 0 && (
          <label>
            Registration
            <select
              value={selected ?? ""}
              onChange={(e) => {
                update(regs, false);
                setSelected(Number(e.target.value));
              }}
            >
              {regs.map((x) => (
                <option key={x.id} value={x.id}>
                  #{x.id}
                  {x.parent ? ` (edit of #${x.parent})` : ""} — {x.result.summary.ok} ok,{" "}
                  {x.result.summary.flagged} flagged
                  {x.applied ? " — applied" : ""}
                </option>
              ))}
            </select>
          </label>
        )}

        {reg && graph && s && (
          <div className="reg-body">
            <svg
              className="reg-graph"
              viewBox={`0 0 ${SIZE} ${SIZE}`}
              role="img"
              aria-label="Registration graph"
            >
              {graph.edges.map((e) => {
                const a = graph.nodes[e.from];
                const b = e.to === null ? { x: a.x, y: a.y - 28 } : graph.nodes[e.to];
                // Fan parallel links out sideways.
                const dx = b.x - a.x;
                const dy = b.y - a.y;
                const len = Math.hypot(dx, dy) || 1;
                // Lanes 0, 1, 2, 3… sit at 0, +6, −6, +12… px.
                const off = (e.lane % 2 ? 1 : -1) * Math.ceil(e.lane / 2) * 6;
                const ox = (-dy / len) * off;
                const oy = (dx / len) * off;
                return (
                  <line
                    key={e.link}
                    x1={a.x + ox}
                    y1={a.y + oy}
                    x2={b.x + ox}
                    y2={b.y + oy}
                    stroke={STROKE[e.style]}
                    strokeWidth={link === e.link ? 5 : 3}
                    strokeDasharray={
                      e.style === "shape-only" || e.style === "deleted" ? "6 4" : undefined
                    }
                    className="reg-edge"
                    onClick={() => setLink(e.link)}
                  >
                    <title>
                      {reg.result.links[e.link].kind} link {e.link}: {e.style}
                    </title>
                  </line>
                );
              })}
              {graph.nodes.map((n) => (
                <g key={n.scan}>
                  <circle
                    cx={n.x}
                    cy={n.y}
                    r={9}
                    className={n.verified ? "reg-node" : "reg-node unverified"}
                  />
                  {/* Labels on the right half go left of the node, so they stay inside. */}
                  <text
                    x={n.x > SIZE / 2 ? n.x - 12 : n.x + 12}
                    y={n.y + 4}
                    textAnchor={n.x > SIZE / 2 ? "end" : "start"}
                    className="reg-label"
                  >
                    {n.name}
                  </text>
                </g>
              ))}
            </svg>
            <div className="reg-side">
              <p>
                {s.links} links: {s.ok} consistent, {s.flagged} flagged, {s.untested} untested
                {s.shape_only ? `, ${s.shape_only} by shape alone` : ""}. Mean target residual{" "}
                {mm(s.target_rms_mean_m)}, largest {mm(s.target_residual_max_m)}.
              </p>
              {s.unverified_scans > 0 && (
                <p className="warn">
                  {s.unverified_scans} scan(s) are placed by shape matching alone (ringed). Check
                  them against the scene before relying on them.
                </p>
              )}
              <p className="muted legend">
                <span style={{ color: STROKE.ok }}>━ consistent</span>{" "}
                <span style={{ color: STROKE.flagged }}>━ flagged</span>{" "}
                <span style={{ color: STROKE.untested }}>━ untested</span>{" "}
                <span style={{ color: STROKE["shape-only"] }}>╍ shape only</span>
              </p>
              {l && r && link !== null ? (
                <div className="reg-link">
                  <strong>
                    {l.kind} link {link}: {scanName(l.a)} – {scanName(l.b)}
                  </strong>
                  <div>
                    {r.status}
                    {l.forced ? " (forced)" : ""}
                    {l.shape_only ? ", start from shape alone" : ""}
                  </div>
                  <div>
                    Residual {mm(r.rms)} RMS, {mm(r.max)} largest, over {l.pairs.length} point pairs
                  </div>
                  <div>
                    χ²/dof {r.chi2_per_dof.toFixed(2)} (limit {r.limit_per_dof.toFixed(2)} at 99.9
                    %)
                  </div>
                  {overlap != null && <div>Overlap {(overlap * 100).toFixed(0)} %</div>}
                  <div className="buttons">
                    <button onClick={() => setDel(toggle(del, link, force))}>
                      {del.has(link) ? "Keep" : "Delete"}
                    </button>
                    <button onClick={() => setForce(toggle(force, link, del))}>
                      {force.has(link) ? "Unforce" : "Force"}
                    </button>
                  </div>
                </div>
              ) : (
                <p className="muted">Click a link for its figures.</p>
              )}
              {(del.size > 0 || force.size > 0) && (
                <button disabled={busy !== null} onClick={() => void resolve()}>
                  Re-solve without {del.size} and forcing {force.size} link(s)
                </button>
              )}
            </div>
          </div>
        )}

        <div className="buttons">
          {applied && (
            <button disabled={busy !== null} onClick={() => void apply(null)}>
              Revert to file poses
            </button>
          )}
          {reg && (
            <button disabled={busy !== null} onClick={() => void exportPdf(reg.id)}>
              Export PDF…
            </button>
          )}
          {reg && !reg.applied && (
            <button className="primary" disabled={busy !== null} onClick={() => void apply(reg.id)}>
              Apply #{reg.id}
            </button>
          )}
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
