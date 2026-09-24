// The guided workflows panel: pick a job, follow its steps. A step ticks itself when the
// project shows it done; optional steps say so. The sample case gives something to practise on.
import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { HelpButton } from "../help/Help";
import { GUIDES, type GuideState } from "./guides";

export function GuidePanel({
  hasProject,
  onNotice,
}: {
  hasProject: boolean;
  onNotice: (m: string | null) => void;
}) {
  const [guide, setGuide] = useState<string>(() => {
    try {
      return localStorage.getItem("locus.guide") ?? "";
    } catch {
      return "";
    }
  });
  const [state, setState] = useState<GuideState | null>(null);
  const g = GUIDES.find((x) => x.id === guide);

  // Follow the project: re-read what it has every few seconds while a guide is open.
  useEffect(() => {
    if (!g) return;
    let live = true;
    const read = () =>
      void api.guideState().then(
        (s) => live && setState(s),
        () => {},
      );
    read();
    const t = setInterval(read, 3000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [g, hasProject]);

  const choose = (id: string) => {
    setGuide(id);
    try {
      localStorage.setItem("locus.guide", id);
    } catch {
      // not remembered, that's all
    }
  };

  const next =
    g && state ? g.steps.findIndex((s) => !s.optional && !s.done?.(state, hasProject)) : -1;

  return (
    <section className="guide">
      <h2>Guides</h2>
      <label>
        <select value={guide} onChange={(e) => choose(e.target.value)}>
          <option value="">Choose a job…</option>
          {GUIDES.map((x) => (
            <option key={x.id} value={x.id}>
              {x.name}
            </option>
          ))}
        </select>
      </label>
      {g && (
        <>
          <p className="muted">{g.summary}</p>
          <ol className="guide-steps">
            {g.steps.map((s, i) => {
              const done = !!state && !!s.done?.(state, hasProject);
              return (
                <li key={i} className={done ? "done" : i === next ? "current" : ""}>
                  <div className="with-help">
                    <strong>
                      {done ? "✓ " : ""}
                      {s.title}
                    </strong>
                    {s.optional && <span className="muted"> (optional)</span>}
                    {s.help && <HelpButton topic={s.help} />}
                  </div>
                  {(i === next || !done) && (
                    <>
                      <p>{s.body}</p>
                      <p className="muted">Where: {s.where}</p>
                    </>
                  )}
                </li>
              );
            })}
          </ol>
          {next === -1 && state && <p className="ok">Every step is done.</p>}
        </>
      )}
      {guide === "indoor" && (
        <button
          onClick={async () => {
            const scripted = (globalThis as { __locusTestPackageDir?: string })
              .__locusTestPackageDir;
            const dir =
              scripted ??
              ((await openDialog({ directory: true, title: "Where to put the sample case" })) as
                string | null);
            if (!dir) return;
            try {
              const made = await api.sampleCreate(dir);
              onNotice(
                `The sample case's evidence is in ${made}: a room scan and its stain photos (synthetic, for practice). Follow the steps above with it.`,
              );
            } catch (e) {
              onNotice(String(e));
            }
          }}
        >
          Create the sample case…
        </button>
      )}
    </section>
  );
}
