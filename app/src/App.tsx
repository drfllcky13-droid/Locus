import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { api, type EvidenceRecord, type ProjectInfo } from "./api";
import { formatBytes, formatCount, integrityProblems, unitSymbol } from "./format";
import { ImportDialog } from "./ImportDialog";
import { ProjectDialog } from "./ProjectDialog";
import { Viewport } from "./viewer3d/Viewport";

const IMPORT_EXTENSIONS = [
  "e57",
  "las",
  "laz",
  "ply",
  "pts",
  "xyz",
  "obj",
  "gltf",
  "glb",
  "jpg",
  "jpeg",
  "png",
];

type Dialog = { kind: "new" } | { kind: "open" } | { kind: "import"; path: string } | null;

export function App() {
  const [project, setProject] = useState<ProjectInfo | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [verifying, setVerifying] = useState(false);

  const startImport = useCallback(async () => {
    if (!project) return setNotice("Create or open a project before importing evidence.");
    const path = await open({
      title: "Import evidence",
      filters: [{ name: "Scans, meshes and photos", extensions: IMPORT_EXTENSIONS }],
    });
    if (typeof path === "string") setDialog({ kind: "import", path });
  }, [project]);

  const verify = useCallback(async () => {
    if (!project) return setNotice("Open a project first.");
    setVerifying(true);
    try {
      const p = await api.evidenceVerify(() => {});
      setProject(p);
      const n = p.integrity?.results.length ?? 0;
      if (p.integrity && integrityProblems(p.integrity).length === 0) {
        setNotice(
          n === 1
            ? "The evidence file matches its recorded SHA-256."
            : `All ${n} evidence files match their recorded SHA-256.`,
        );
      } else {
        setNotice(null); // the panel's integrity warning lists the problems
      }
    } catch (e) {
      setNotice(String(e));
    } finally {
      setVerifying(false);
    }
  }, [project]);

  useEffect(() => {
    const unlisten = listen<string>("menu", ({ payload }) => {
      if (payload === "new_project") setDialog({ kind: "new" });
      else if (payload === "open_project") setDialog({ kind: "open" });
      else if (payload === "import") void startImport();
      else if (payload === "verify_evidence") void verify();
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [startImport, verify]);

  return (
    <div className="app">
      <aside className="sidebar">
        {project ? (
          <ProjectPanel
            project={project}
            onImport={startImport}
            onVerify={verify}
            verifying={verifying}
          />
        ) : (
          <div className="welcome">
            <h1>Locus</h1>
            <p className="muted">Create a project, or open an existing .locus folder.</p>
            <button className="primary" onClick={() => setDialog({ kind: "new" })}>
              New project…
            </button>
            <button onClick={() => setDialog({ kind: "open" })}>Open project…</button>
          </div>
        )}
      </aside>
      <main className="main">
        <Viewport />
      </main>

      {(dialog?.kind === "new" || dialog?.kind === "open") && (
        <ProjectDialog
          mode={dialog.kind}
          onClose={() => setDialog(null)}
          onOpened={(p) => {
            setProject(p);
            setDialog(null);
            setNotice(null);
          }}
        />
      )}
      {dialog?.kind === "import" && (
        <ImportDialog
          path={dialog.path}
          onClose={() => setDialog(null)}
          onImported={(p, warning) => {
            setProject(p);
            setDialog(null);
            setNotice(warning);
          }}
        />
      )}
      {notice && (
        <div className="notice" role="status">
          <span>{notice}</span>
          <button onClick={() => setNotice(null)} aria-label="Dismiss">
            ×
          </button>
        </div>
      )}
    </div>
  );
}

function ProjectPanel({
  project,
  onImport,
  onVerify,
  verifying,
}: {
  project: ProjectInfo;
  onImport: () => void;
  onVerify: () => void;
  verifying: boolean;
}) {
  return (
    <>
      <header className="project">
        <h1>{project.name}</h1>
        <p className="muted">Examiner: {project.examiner}</p>
        <p className="muted" title={project.audit_head}>
          Audit log: {formatCount(project.audit_entries, "entry", "entries")}, head{" "}
          <code>{project.audit_head.slice(0, 12)}</code>
        </p>
        <div className="row">
          <button className="primary" onClick={onImport}>
            Import evidence…
          </button>
          <button onClick={onVerify} disabled={verifying || project.evidence.length === 0}>
            {verifying ? "Verifying…" : "Verify evidence"}
          </button>
        </div>
      </header>
      {project.integrity && <IntegrityWarning problems={integrityProblems(project.integrity)} />}
      <h2>Evidence</h2>
      {project.evidence.length === 0 ? (
        <p className="muted">No evidence imported yet.</p>
      ) : (
        <ul className="evidence">
          {project.evidence.map((e) => (
            <EvidenceItem key={e.id} e={e} />
          ))}
        </ul>
      )}
    </>
  );
}

function IntegrityWarning({ problems }: { problems: string[] }) {
  if (problems.length === 0) return null;
  return (
    <div className="integrity" role="alert">
      <strong>Evidence integrity check failed</strong>
      <ul>
        {problems.map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
      <p>
        The evidence folder no longer matches the hashes recorded at import. This finding is in the
        audit log. Restore the original files from a verified copy before relying on this project.
      </p>
    </div>
  );
}

function EvidenceItem({ e }: { e: EvidenceRecord }) {
  const c = e.contents;
  const file = e.original_path.split(/[\\/]/).pop();
  const parts = [
    c.scans.length > 0 &&
      formatCount(
        c.scans.reduce((n, s) => n + s.point_count, 0),
        "point",
      ),
    c.scans.length > 1 && formatCount(c.scans.length, "scan"),
    c.meshes.length > 0 && formatCount(c.meshes.length, "mesh", "meshes"),
    c.images.length > 0 && formatCount(c.images.length, "image"),
    e.unit && `unit ${unitSymbol(e.unit)}`,
  ].filter(Boolean);
  return (
    <li>
      <div className="title">
        <span className="id">#{e.id}</span> {file} <span className="muted">{c.format}</span>
      </div>
      <div className="muted">{parts.join(" · ")}</div>
      <div className="muted">{formatBytes(e.size)}</div>
      <code className="hash" title="SHA-256">
        {e.sha256}
      </code>
      <div className="muted">
        Imported {new Date(e.imported_at).toLocaleString()} by {e.imported_by}
      </div>
      {c.warnings.length > 0 && (
        <div className="warn">{formatCount(c.warnings.length, "warning")}</div>
      )}
    </li>
  );
}
