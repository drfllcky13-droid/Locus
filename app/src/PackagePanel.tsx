// The viewer's side panel in a case package: what the package is, whether every file still
// matches its manifest, and its reports and videos to open.
import { HelpButton } from "./help/Help";
import { api } from "./api";
import type { PackageInfo } from "./readOnly";

export function PackagePanel({
  pkg,
  onNotice,
}: {
  pkg: PackageInfo;
  onNotice: (m: string | null) => void;
}) {
  const m = pkg.check.manifest;
  const files = (dir: string) => m.files.filter((f) => f.path.startsWith(`${dir}/`));
  const open = (path: string) => api.packageFileOpen(path).catch((e) => onNotice(String(e)));
  const list = (dir: string, title: string) =>
    files(dir).length > 0 && (
      <>
        <h3>{title}</h3>
        <ul className="package-files">
          {files(dir).map((f) => (
            <li key={f.path}>
              <button className="link" onClick={() => void open(f.path)}>
                {f.path.slice(dir.length + 1)}
              </button>
            </li>
          ))}
        </ul>
      </>
    );
  return (
    <section className="package">
      <h2 className="with-help">
        Case package (read-only) <HelpButton topic="case-package" />
      </h2>
      {pkg.check.problems.length === 0 ? (
        <p className="ok">All {pkg.check.checked} files match the package manifest.</p>
      ) : (
        <div className="integrity-warning">
          <strong>This package has been changed since it was made:</strong>
          <ul>
            {pkg.check.problems.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
        </div>
      )}
      <p className="muted">
        {m.project}
        {m.case_number ? ` · case ${m.case_number}` : ""}. Made by {m.made_by}, {m.made_at}. Package
        hash (Help → About): <span className="hash">{pkg.check.hash}</span>
      </p>
      {m.not_included.length > 0 && (
        <p className="muted">Not included: {m.not_included.join("; ")}.</p>
      )}
      {list("reports", "Reports")}
      {list("videos", "Videos")}
      <p className="muted">
        Measurements made here are for this session only: nothing in the package can be changed.
      </p>
    </section>
  );
}
