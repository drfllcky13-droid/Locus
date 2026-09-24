// Writing out of the project: the case report, measurements as CSV, point clouds as E57, LAS
// or LAZ, and diagrams as PDF, PNG, TIFF or DXF. Every file is hashed and logged in the audit
// log (src-tauri/src/export_cmds.rs). The 3D scene's glTF export is in the scene builder.
import { useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api, type DiagramRevision } from "../api";
import type { ScanData } from "../viewer3d/pointcloud";

/** In-app test scripts can't answer the native dialog; they set the path it would return. */
export async function savePath(name: string, ext: string, label: string) {
  const scripted = (globalThis as { __locusTestSavePath?: string }).__locusTestSavePath;
  return (
    scripted ??
    (await saveDialog({
      defaultPath: `${name}.${ext}`,
      filters: [{ name: label, extensions: [ext] }],
    }))
  );
}

export function ExportPanel({
  scans,
  diagrams,
  onNotice,
}: {
  scans: ScanData[];
  diagrams: DiagramRevision[];
  onNotice: (m: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const [cloud, setCloud] = useState<"e57" | "las" | "laz">("e57");
  const [which, setWhich] = useState<string>("");
  const [diagram, setDiagram] = useState<number | "">("");
  const [scale, setScale] = useState(100);
  const [paper, setPaper] = useState<"A4" | "A3">("A4");
  const [landscape, setLandscape] = useState(true);
  const [format, setFormat] = useState<"pdf" | "png" | "tiff" | "dxf">("pdf");
  const [dpi, setDpi] = useState(300);
  const [busy, setBusy] = useState(false);

  const run = async (label: string, f: () => Promise<string | null>) => {
    setBusy(true);
    try {
      const sha = await f();
      if (sha) onNotice(`${label} written (SHA-256 ${sha}; recorded in the audit log).`);
    } catch (e) {
      onNotice(String(e));
    } finally {
      setBusy(false);
    }
  };
  const d = diagrams.find((x) => x.document_id === diagram);

  return (
    <section className="panel-section">
      <h3>Export</h3>
      {!open && <button onClick={() => setOpen(true)}>Export files…</button>}
      {open && (
        <>
          <p className="muted">
            Every file is written from saved records, hashed, and recorded in the audit log.
          </p>
          <div className="row">
            <button
              disabled={busy}
              onClick={() =>
                run("Case report", async () => {
                  const path = await savePath("Case report", "pdf", "PDF");
                  return path ? api.caseReport(path) : null;
                })
              }
            >
              Case report…
            </button>
            <button
              disabled={busy}
              onClick={() =>
                run("Measurements", async () => {
                  const path = await savePath("Measurements", "csv", "CSV");
                  return path ? api.measurementsCsv(path) : null;
                })
              }
            >
              Measurements CSV…
            </button>
          </div>

          <h4>Point clouds</h4>
          <label>
            Scans
            <select value={which} onChange={(e) => setWhich(e.target.value)}>
              <option value="">All ({scans.length})</option>
              {scans.map((s) => (
                <option key={s.key} value={s.key}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
          <div className="row">
            <select value={cloud} onChange={(e) => setCloud(e.target.value as typeof cloud)}>
              <option value="e57">E57</option>
              <option value="las">LAS</option>
              <option value="laz">LAZ (compressed LAS)</option>
            </select>
            <button
              disabled={busy || !scans.length}
              onClick={() =>
                run("Point cloud", async () => {
                  const path = await savePath("Point cloud", cloud, cloud.toUpperCase());
                  return path ? api.pointcloudExport(which ? [which] : [], cloud, path) : null;
                })
              }
            >
              Export…
            </button>
          </div>
          <p className="muted">
            Project frame, metres; points removed by cleanup are left out. LAS and LAZ keep 0.1 mm
            steps and carry no coordinate reference system.
          </p>

          <h4>Diagrams</h4>
          <label>
            Diagram
            <select
              value={diagram}
              onChange={(e) => setDiagram(e.target.value ? Number(e.target.value) : "")}
            >
              <option value="">Choose…</option>
              {diagrams.map((x) => (
                <option key={x.document_id} value={x.document_id}>
                  {x.name} (rev. {x.number})
                </option>
              ))}
            </select>
          </label>
          <label>
            Format
            <select value={format} onChange={(e) => setFormat(e.target.value as typeof format)}>
              <option value="pdf">PDF (to scale)</option>
              <option value="png">PNG (to scale)</option>
              <option value="tiff">TIFF (to scale)</option>
              <option value="dxf">DXF (world metres)</option>
            </select>
          </label>
          {format !== "dxf" && (
            <div className="row">
              <label>
                Scale 1:
                <input
                  type="number"
                  value={scale}
                  min={1}
                  onChange={(e) => e.target.valueAsNumber > 0 && setScale(e.target.valueAsNumber)}
                />
              </label>
              <label>
                Paper
                <select value={paper} onChange={(e) => setPaper(e.target.value as "A4" | "A3")}>
                  <option>A4</option>
                  <option>A3</option>
                </select>
              </label>
              <label className="inline">
                <input
                  type="checkbox"
                  checked={landscape}
                  onChange={(e) => setLandscape(e.target.checked)}
                />
                Landscape
              </label>
            </div>
          )}
          {(format === "png" || format === "tiff") && (
            <label>
              Resolution (dots per inch)
              <select value={dpi} onChange={(e) => setDpi(Number(e.target.value))}>
                {[150, 300, 600].map((x) => (
                  <option key={x} value={x}>
                    {x}
                  </option>
                ))}
              </select>
            </label>
          )}
          <button
            disabled={busy || !d}
            onClick={() =>
              d &&
              run(`Diagram ${format.toUpperCase()}`, async () => {
                const path = await savePath(
                  d.name,
                  format === "tiff" ? "tif" : format,
                  format.toUpperCase(),
                );
                if (!path) return null;
                if (format === "pdf")
                  return api.diagramPdf(d.document_id, scale, paper, landscape, path);
                if (format === "dxf") return api.diagramDxf(d.document_id, path);
                return api.diagramImage(d.document_id, scale, paper, landscape, dpi, format, path);
              })
            }
          >
            Export…
          </button>
        </>
      )}
    </section>
  );
}
