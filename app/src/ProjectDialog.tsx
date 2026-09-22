import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { api, type ProjectInfo } from "./api";
import { formatBytes } from "./format";

const EXAMINER_KEY = "locus.examiner";

function rememberedExaminer(): string {
  try {
    return localStorage.getItem(EXAMINER_KEY) ?? "";
  } catch {
    return "";
  }
}

export function ProjectDialog({
  mode,
  onClose,
  onOpened,
}: {
  mode: "new" | "open";
  onClose: () => void;
  onOpened: (p: ProjectInfo) => void;
}) {
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const [examiner, setExaminer] = useState(rememberedExaminer);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [hashed, setHashed] = useState(0);

  const choose = async () => {
    const picked = await open({
      directory: true,
      title: mode === "new" ? "Where to create the project" : "Project folder (.locus)",
    });
    if (typeof picked === "string") setFolder(picked);
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const p =
        mode === "new"
          ? await api.projectCreate(folder, name, examiner)
          : await api.projectOpen(folder, examiner, setHashed);
      try {
        localStorage.setItem(EXAMINER_KEY, examiner.trim());
      } catch {
        // Remembering the name is a convenience only.
      }
      onOpened(p);
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  const ready = folder && examiner.trim() && (mode === "open" || name.trim());
  return (
    <div className="overlay">
      <form
        className="dialog"
        onSubmit={submit}
        aria-label={mode === "new" ? "New project" : "Open project"}
      >
        <h2>{mode === "new" ? "New project" : "Open project"}</h2>
        {mode === "new" && (
          <label>
            Project name
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
              placeholder="e.g. Case 2026-0142"
            />
          </label>
        )}
        <label>
          {mode === "new" ? "Location" : "Project folder"}
          <span className="row">
            <input value={folder} readOnly placeholder="No folder chosen" />
            <button type="button" onClick={choose}>
              Choose…
            </button>
          </span>
        </label>
        {mode === "new" && folder && name.trim() && (
          <p className="muted">
            Creates {folder}
            {folder.includes("\\") ? "\\" : "/"}
            {name.trim()}.locus
          </p>
        )}
        <label>
          Examiner
          <input
            value={examiner}
            onChange={(e) => setExaminer(e.target.value)}
            placeholder="Your name"
          />
        </label>
        <p className="muted">
          The examiner&apos;s name is recorded in the audit log with every change.
        </p>
        {busy && mode === "open" && (
          <p className="muted">
            Checking every evidence file against its recorded SHA-256… {formatBytes(hashed)}
          </p>
        )}
        {error && <p className="error">{error}</p>}
        <div className="buttons">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button className="primary" type="submit" disabled={!ready || busy}>
            {mode === "new" ? "Create" : "Open"}
          </button>
        </div>
      </form>
    </div>
  );
}
