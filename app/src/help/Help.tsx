// Contextual help: the method notes (docs/methods/*.md), bundled at build time so they work
// offline, opened from a "?" beside each tool. A small Markdown reader: headings, paragraphs,
// lists, tables, and bold, italic, code and links inline.
import { useState, type ReactNode } from "react";

const NOTES = import.meta.glob("../../../docs/methods/*.md", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

export type Topic =
  | "animation"
  | "audit-log"
  | "bloodstain"
  | "camera-height"
  | "case-package"
  | "cleanup"
  | "crash"
  | "crash-edr"
  | "crash-stiffness"
  | "crash-volume"
  | "diagrams"
  | "exports"
  | "hand-measurements"
  | "licensing"
  | "measurement"
  | "photogrammetry"
  | "registration"
  | "scene3d"
  | "trajectory"
  | "validation"
  | "validation-protocol";

export function note(topic: Topic): string | undefined {
  return Object.entries(NOTES).find(([path]) => path.endsWith(`/${topic}.md`))?.[1];
}

/** Inline Markdown: **bold**, *italic*, `code`, [text](link). */
function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|\*[^*\s][^*]*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;
  let last = 0;
  let k = 0;
  for (const m of text.matchAll(re)) {
    if (m.index! > last) out.push(text.slice(last, m.index));
    const t = m[0];
    if (t.startsWith("**")) out.push(<strong key={k++}>{t.slice(2, -2)}</strong>);
    else if (t.startsWith("`")) out.push(<code key={k++}>{t.slice(1, -1)}</code>);
    else if (t.startsWith("[")) out.push(<em key={k++}>{t.slice(1, t.indexOf("]"))}</em>);
    else out.push(<em key={k++}>{t.slice(1, -1)}</em>);
    last = m.index! + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Block Markdown to React: enough for the method notes. */
export function Markdown({ text }: { text: string }) {
  const lines = text.replace(/\r/g, "").split("\n");
  const out: ReactNode[] = [];
  let i = 0;
  let k = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (!l.trim()) {
      i++;
      continue;
    }
    const h = l.match(/^(#{1,4})\s+(.*)/);
    if (h) {
      const level = Math.min(h[1].length + 1, 5);
      const Tag = `h${level}` as "h2";
      out.push(<Tag key={k++}>{inline(h[2])}</Tag>);
      i++;
      continue;
    }
    if (l.trim().startsWith("|")) {
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        const cells = lines[i]
          .trim()
          .slice(1, -1)
          .split("|")
          .map((c) => c.trim());
        if (!cells.every((c) => /^:?-+:?$/.test(c))) rows.push(cells);
        i++;
      }
      out.push(
        <table key={k++} className="help-table">
          <thead>
            <tr>
              {rows[0]?.map((c, j) => (
                <th key={j}>{inline(c)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.slice(1).map((r, ri) => (
              <tr key={ri}>
                {r.map((c, j) => (
                  <td key={j}>{inline(c)}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>,
      );
      continue;
    }
    if (/^\s*([-*]|\d+\.)\s/.test(l)) {
      const items: { depth: number; text: string }[] = [];
      while (i < lines.length && /^\s*([-*]|\d+\.)\s/.test(lines[i])) {
        const m = lines[i].match(/^(\s*)([-*]|\d+\.)\s+(.*)/)!;
        let t = m[3];
        i++;
        // Continuation lines, indented under the item.
        while (
          i < lines.length &&
          lines[i].trim() &&
          /^\s{2,}\S/.test(lines[i]) &&
          !/^\s*([-*]|\d+\.)\s/.test(lines[i])
        ) {
          t += " " + lines[i].trim();
          i++;
        }
        items.push({ depth: Math.floor(m[1].length / 2), text: t });
      }
      out.push(
        <ul key={k++} className="help-list">
          {items.map((it, j) => (
            <li key={j} style={{ marginLeft: `${it.depth * 16}px` }}>
              {inline(it.text)}
            </li>
          ))}
        </ul>,
      );
      continue;
    }
    // A paragraph: lines up to a blank or another block.
    let p = l.trim();
    i++;
    while (
      i < lines.length &&
      lines[i].trim() &&
      !/^(#{1,4}\s|\s*([-*]|\d+\.)\s|\s*\|)/.test(lines[i])
    ) {
      p += " " + lines[i].trim();
      i++;
    }
    out.push(<p key={k++}>{inline(p)}</p>);
  }
  return <>{out}</>;
}

/** A "?" that opens the method note for `topic`. */
export function HelpButton({ topic, label }: { topic: Topic; label?: string }) {
  const [open, setOpen] = useState(false);
  const text = note(topic);
  if (!text) return null;
  return (
    <>
      <button
        className="help-button"
        title={label ?? "How this works (method note)"}
        aria-label={label ?? "Help"}
        onClick={() => setOpen(true)}
      >
        ?
      </button>
      {open && (
        <div className="overlay" onClick={() => setOpen(false)}>
          <div
            className="dialog help-dialog"
            role="dialog"
            aria-label="Method note"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="help-body">
              <Markdown text={text} />
            </div>
            <div className="buttons">
              <button onClick={() => setOpen(false)}>Close</button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
