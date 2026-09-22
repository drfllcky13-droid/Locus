import { useEffect, useRef, useState } from "react";
import { api, type LinearUnit, type Preview, type ProjectInfo, type Progress } from "./api";
import {
  UNITS,
  formatBytes,
  formatCount,
  formatExtent,
  needsUnit,
  progressPercent,
  unitSymbol,
} from "./format";

type Phase =
  | { kind: "reading"; progress: Progress | null }
  | { kind: "review"; preview: Preview }
  | { kind: "committing"; preview: Preview; progress: Progress | null }
  | { kind: "error"; message: string };

const STAGE_LABEL: Record<Progress["stage"], string> = {
  reading: "Reading file",
  points: "Reading points",
  hashing: "Computing SHA-256",
  copying: "Copying into project",
};

function ProgressBar({ progress }: { progress: Progress | null }) {
  if (!progress) return <p className="muted">Starting…</p>;
  const pct = progressPercent(progress.done, progress.total);
  const amount =
    progress.stage === "points"
      ? `${formatCount(progress.done, "point")} of ${progress.total.toLocaleString("en-US")}`
      : `${formatBytes(progress.done)}`;
  return (
    <div>
      <p>
        {STAGE_LABEL[progress.stage]}: {pct}% ({amount})
      </p>
      <progress max={100} value={pct} />
    </div>
  );
}

export function ImportDialog({
  path,
  onClose,
  onImported,
}: {
  path: string;
  onClose: () => void;
  onImported: (project: ProjectInfo, warning: string | null) => void;
}) {
  const [phase, setPhase] = useState<Phase>({ kind: "reading", progress: null });
  const [unit, setUnit] = useState<LinearUnit | "">("");
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return; // StrictMode runs effects twice; preview once.
    started.current = true;
    api
      .importPreview(path, (progress) => setPhase({ kind: "reading", progress }))
      .then((preview) => setPhase({ kind: "review", preview }))
      .catch((e) => setPhase({ kind: "error", message: String(e) }));
  }, [path]);

  const commit = (preview: Preview) => {
    setPhase({ kind: "committing", preview, progress: null });
    api
      .importCommit(preview.sha256, unit || null, (progress) =>
        setPhase({ kind: "committing", preview, progress }),
      )
      .then((r) => onImported(r.project, r.warning))
      .catch((e) => setPhase({ kind: "error", message: String(e) }));
  };

  return (
    <div className="overlay">
      <div className="dialog wide" role="dialog" aria-label="Import evidence">
        <h2>Import evidence</h2>
        <p className="path">{path}</p>
        {phase.kind === "reading" && <ProgressBar progress={phase.progress} />}
        {phase.kind === "committing" && <ProgressBar progress={phase.progress} />}
        {phase.kind === "error" && (
          <>
            <p className="error">{phase.message}</p>
            <div className="buttons">
              <button onClick={onClose}>Close</button>
            </div>
          </>
        )}
        {phase.kind === "review" && (
          <Review preview={phase.preview} unit={unit} setUnit={setUnit}>
            <div className="buttons">
              <button onClick={onClose}>Cancel</button>
              <button
                className="primary"
                disabled={needsUnit(phase.preview.contents) && !unit}
                onClick={() => commit(phase.preview)}
              >
                Import as evidence
              </button>
            </div>
          </Review>
        )}
        <p className="help">
          Files are opened read-only. Locus copies them into the project, checks the copy against
          the hash above, and never changes the original. Native scanner project files use
          proprietary formats that Locus does not read: export the scans to E57 from the
          scanner&apos;s own software, then import the E57.
        </p>
      </div>
    </div>
  );
}

function Review({
  preview,
  unit,
  setUnit,
  children,
}: {
  preview: Preview;
  unit: LinearUnit | "";
  setUnit: (u: LinearUnit | "") => void;
  children: React.ReactNode;
}) {
  const c = preview.contents;
  const shownUnit = c.declared_unit ?? (unit || null);
  return (
    <>
      <dl className="facts">
        <dt>SHA-256</dt>
        <dd>
          <code className="hash">{preview.sha256}</code>
        </dd>
        <dt>Size</dt>
        <dd>{formatBytes(preview.size)}</dd>
        <dt>Format</dt>
        <dd>{c.format}</dd>
        {c.scans.length > 0 && (
          <>
            <dt>Points</dt>
            <dd>
              {formatCount(
                c.scans.reduce((n, s) => n + s.point_count, 0),
                "point",
              )}{" "}
              in {formatCount(c.scans.length, "scan")}
            </dd>
          </>
        )}
        {c.crs && (
          <>
            <dt>Coordinate system</dt>
            <dd className="crs">{c.crs}</dd>
          </>
        )}
        <dt>Unit</dt>
        <dd>
          {c.declared_unit ? (
            `${UNITS.find((u) => u.value === c.declared_unit)?.label} (stated by the file)`
          ) : needsUnit(c) ? (
            <label>
              <select
                value={unit}
                onChange={(e) => setUnit(e.target.value as LinearUnit | "")}
                aria-label="Unit of the file's coordinates"
              >
                <option value="">Choose the unit of this file&apos;s coordinates…</option>
                {UNITS.map((u) => (
                  <option key={u.value} value={u.value}>
                    {u.label}
                  </option>
                ))}
              </select>{" "}
              <span className="muted">
                This file doesn&apos;t say. Your choice is audit-logged.
              </span>
            </label>
          ) : (
            "not applicable"
          )}
        </dd>
        {c.y_up && (
          <>
            <dt>Axes</dt>
            <dd>Y-up (converted to the project&apos;s Z-up frame when placed)</dd>
          </>
        )}
      </dl>

      {c.scans.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>Scan</th>
              <th>Points</th>
              <th>No position</th>
              <th>Extent (x × y × z)</th>
              <th>Attributes</th>
            </tr>
          </thead>
          <tbody>
            {c.scans.map((s, i) => (
              <tr key={i}>
                <td>{s.name}</td>
                <td className="num">{formatCount(s.point_count, "point")}</td>
                <td className="num">{formatCount(s.invalid_points, "point")}</td>
                <td className="num">{formatExtent(s.bounds, shownUnit)}</td>
                <td>{s.attributes.join(", ") || "none"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {c.meshes.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>Mesh</th>
              <th>Vertices</th>
              <th>Faces</th>
              <th>Extent (x × y × z)</th>
            </tr>
          </thead>
          <tbody>
            {c.meshes.map((m, i) => (
              <tr key={i}>
                <td>{m.name}</td>
                <td className="num">{formatCount(m.vertex_count, "vertex", "vertices")}</td>
                <td className="num">{formatCount(m.face_count, "face")}</td>
                <td className="num">{formatExtent(m.bounds, shownUnit)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {c.images.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>Image</th>
              <th>Type</th>
              <th>Size</th>
              <th>Scan</th>
              <th>EXIF</th>
            </tr>
          </thead>
          <tbody>
            {c.images.map((img, i) => (
              <tr key={i}>
                <td>{img.name}</td>
                <td>{img.kind.replace("_", " ")}</td>
                <td className="num">
                  {img.width} × {img.height} px
                </td>
                <td>{img.scan === null ? "—" : c.scans[img.scan]?.name}</td>
                <td>
                  {img.exif.length === 0 ? (
                    "none"
                  ) : (
                    <details>
                      <summary>{formatCount(img.exif.length, "field")}</summary>
                      <dl className="exif">
                        {img.exif.map((f, j) => (
                          <div key={j}>
                            <dt>{f.tag}</dt>
                            <dd>{f.value}</dd>
                          </div>
                        ))}
                      </dl>
                    </details>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {c.warnings.length > 0 && (
        <ul className="warnings">
          {c.warnings.map((w, i) => (
            <li key={i}>{w}</li>
          ))}
        </ul>
      )}
      {shownUnit && c.scans.length + c.meshes.length > 0 && (
        <p className="muted">Extents are in {unitSymbol(shownUnit)}, before any scan pose.</p>
      )}
      {children}
    </>
  );
}
