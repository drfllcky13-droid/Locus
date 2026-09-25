import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { api, type DiagramRevision, type EvidenceRecord, type ProjectInfo } from "./api";
import { formatBytes, formatCount, integrityProblems, unitSymbol } from "./format";
import { AboutDialog } from "./AboutDialog";
import { ImportDialog } from "./ImportDialog";
import { ProjectDialog } from "./ProjectDialog";
import { RegistrationDialog } from "./registration/RegistrationDialog";
import { DiagramEditor } from "./diagram2d/DiagramEditor";
import { emptyDiagram } from "./diagram2d/model";
import { Viewport } from "./viewer3d/Viewport";
import { PackagePanel } from "./PackagePanel";
import { GuidePanel } from "./guide/GuidePanel";
import { ReadOnly, type PackageInfo } from "./readOnly";
import { allows, License, type LicenseInfo } from "./license";

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
  "mp4",
  "m4v",
  "mov",
  "avi",
];

type Dialog =
  | { kind: "new" }
  | { kind: "open" }
  | { kind: "about" }
  | { kind: "register" }
  | { kind: "import"; path: string }
  | null;

/** Between crash reports shown together. */
const SEP = "\n\n";

export function App() {
  const [project, setProject] = useState<ProjectInfo | null>(null);
  // Started from a case package: the case opens read-only.
  const [pkg, setPkg] = useState<PackageInfo | null>(null);
  const [license, setLicense] = useState<LicenseInfo | null>(null);
  // Crash reports from an earlier session, shown on request; never sent by the app.
  const [crashes, setCrashes] = useState<{ name: string; text: string }[]>([]);
  const [showCrashes, setShowCrashes] = useState(false);
  useEffect(() => {
    void api.crashReports().then(setCrashes);
    const onError = (e: ErrorEvent) =>
      void api.crashReportView(String(e.message), String(e.error?.stack ?? ""));
    const onRejection = (e: PromiseRejectionEvent) =>
      void api.crashReportView(String(e.reason), String(e.reason?.stack ?? ""));
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, []);
  useEffect(() => {
    void api.licenseInfo().then(setLicense);
  }, []);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // What the main area shows: the 3D scene, or a diagram by id. The diagram list is kept with
  // the project it was loaded for, so a stale list never shows for another project.
  const [tab, setTab] = useState<"scene" | number>("scene");
  const [loaded, setLoaded] = useState<{ root: string; list: DiagramRevision[] } | null>(null);
  const diagrams = project && loaded?.root === project.root ? loaded.list : [];
  const setDiagrams = (f: (ds: DiagramRevision[]) => DiagramRevision[]) =>
    setLoaded((l) => (l ? { ...l, list: f(l.list) } : l));
  const shown = tab !== "scene" && diagrams.some((d) => d.document_id === tab) ? tab : "scene";

  useEffect(() => {
    if (!project) return;
    const root = project.root;
    void api
      .diagrams()
      .then((list) => setLoaded({ root, list }))
      .catch((e) => setNotice(String(e)));
  }, [project]);

  const newDiagram = async () => {
    try {
      const d = await api.diagramCreate(`Diagram ${diagrams.length + 1}`, emptyDiagram());
      setDiagrams((ds) => [...ds, d]);
      setTab(d.document_id);
    } catch (e) {
      setNotice(String(e));
    }
  };
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
    void api.startup().then(async (s) => {
      if (s.package) {
        try {
          const o = await api.packageOpen(() => {});
          setPkg({ folder: o.folder, check: o.check });
          setProject(o.project);
        } catch (e) {
          setNotice(String(e));
        }
        return;
      }
      if (!s.open || !s.examiner) return;
      try {
        setProject(await api.projectOpen(s.open, s.examiner, () => {}));
      } catch (e) {
        setNotice(String(e));
      }
    });
  }, []);

  // Evidence added by the backend itself (a photogrammetry point cloud).
  useEffect(() => {
    const unlisten = listen<ProjectInfo>("project-changed", ({ payload }) => setProject(payload));
    return () => void unlisten.then((f) => f());
  }, []);

  useEffect(() => {
    const unlisten = listen<string>("menu", ({ payload }) => {
      // A case package can't be changed or swapped for another project.
      if (pkg && payload !== "about") return;
      if (payload === "new_project") setDialog({ kind: "new" });
      else if (payload === "open_project") setDialog({ kind: "open" });
      else if (payload === "import") void startImport();
      else if (payload === "verify_evidence") void verify();
      else if (payload === "about") setDialog({ kind: "about" });
      else if (payload === "register") {
        if (license && !allows(license.tier, "registration"))
          setNotice(
            `Scan registration needs the Analyst Plus licence (this is ${license.tier_name}).`,
          );
        else setDialog({ kind: "register" });
      }
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [startImport, verify, pkg, license]);

  return (
    <License.Provider value={license}>
      <ReadOnly.Provider value={!!pkg}>
        <div className="app">
          <aside className="sidebar">
            {pkg && <PackagePanel pkg={pkg} onNotice={setNotice} />}
            {license?.status === "invalid" && !pkg && (
              <p className="muted license-note">
                Licence not valid ({license.problem}): every tool is on without it.
              </p>
            )}
            {project ? (
              <ProjectPanel
                project={project}
                onImport={startImport}
                onVerify={verify}
                verifying={verifying}
                readOnly={!!pkg}
              />
            ) : (
              <div className="welcome">
                <h1>Lotus</h1>
                <p className="muted">Create a project, or open an existing .locus folder.</p>
                <button className="primary" onClick={() => setDialog({ kind: "new" })}>
                  New project…
                </button>
                <button onClick={() => setDialog({ kind: "open" })}>Open project…</button>
              </div>
            )}
            {!pkg && <GuidePanel hasProject={!!project} onNotice={setNotice} />}
          </aside>
          <main className="main">
            {project && (
              <nav className="tabs" aria-label="Views">
                <button
                  className={shown === "scene" ? "tab active" : "tab"}
                  onClick={() => setTab("scene")}
                >
                  3D scene
                </button>
                {!pkg &&
                  diagrams.map((d) => (
                    <button
                      key={d.document_id}
                      className={shown === d.document_id ? "tab active" : "tab"}
                      onClick={() => setTab(d.document_id)}
                    >
                      {d.name}
                    </button>
                  ))}
                {!pkg && (
                  <button className="tab" onClick={() => void newDiagram()}>
                    + New diagram
                  </button>
                )}
              </nav>
            )}
            {/* The 3D view stays mounted, so its point clouds don't reload on every switch. */}
            <div className="main-view" style={{ display: shown === "scene" ? undefined : "none" }}>
              <Viewport
                project={project}
                diagrams={diagrams}
                active={shown === "scene"}
                onNotice={setNotice}
              />
            </div>
            {shown !== "scene" &&
              diagrams
                .filter((d) => d.document_id === shown)
                .map((d) => (
                  <DiagramEditor
                    key={d.document_id}
                    initial={d}
                    onNotice={setNotice}
                    onSaved={(r) =>
                      setDiagrams((ds) => ds.map((x) => (x.document_id === r.document_id ? r : x)))
                    }
                  />
                ))}
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
          {dialog?.kind === "about" && (
            <AboutDialog
              onClose={() => setDialog(null)}
              packageHash={pkg?.check.hash}
              license={license}
              onLicense={setLicense}
            />
          )}
          {dialog?.kind === "register" && project && (
            <RegistrationDialog onClose={() => setDialog(null)} onNotice={setNotice} />
          )}
          {crashes.length > 0 && !showCrashes && (
            <div className="notice" role="status">
              <span>
                Lotus stopped unexpectedly before. A crash report was saved on this computer (no
                case data, nothing sent).
              </span>
              <button onClick={() => setShowCrashes(true)}>View</button>
              <button onClick={() => void api.crashReportsClear().then(() => setCrashes([]))}>
                Dismiss
              </button>
            </div>
          )}
          {showCrashes && (
            <div className="overlay" onClick={() => setShowCrashes(false)}>
              <div
                className="dialog"
                role="dialog"
                aria-label="Crash reports"
                onClick={(e) => e.stopPropagation()}
              >
                <h2>Crash reports</h2>
                <p className="muted">
                  Saved on this computer only. They hold no case data: file paths are removed. To
                  help fix the problem, copy a report and send it to the developer yourself.
                </p>
                <pre className="notices">{crashes.map((c) => c.text).join(SEP)}</pre>
                <div className="buttons">
                  <button
                    onClick={() =>
                      void navigator.clipboard.writeText(crashes.map((c) => c.text).join(SEP))
                    }
                  >
                    Copy
                  </button>
                  <button
                    onClick={() =>
                      void api.crashReportsClear().then(() => {
                        setCrashes([]);
                        setShowCrashes(false);
                      })
                    }
                  >
                    Delete them
                  </button>
                  <button onClick={() => setShowCrashes(false)}>Close</button>
                </div>
              </div>
            </div>
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
      </ReadOnly.Provider>
    </License.Provider>
  );
}

function ProjectPanel({
  project,
  onImport,
  onVerify,
  verifying,
  readOnly,
}: {
  project: ProjectInfo;
  onImport: () => void;
  onVerify: () => void;
  verifying: boolean;
  readOnly: boolean;
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
        {!readOnly && (
          <div className="row">
            <button className="primary" onClick={onImport}>
              Import evidence…
            </button>
            <button onClick={onVerify} disabled={verifying || project.evidence.length === 0}>
              {verifying ? "Verifying…" : "Verify evidence"}
            </button>
          </div>
        )}
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
