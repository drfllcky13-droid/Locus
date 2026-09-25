import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

/**
 * A panel section's title, with a button that folds the section away (the title stays). The
 * section (the heading's parent) gets the `collapsed` class; styles.css hides the rest of it.
 * Sections start open, so what a section shows is unchanged until the examiner folds it.
 */
export function PanelHeader({ level = 3, children }: { level?: 2 | 3; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  const ref = useRef<HTMLHeadingElement>(null);
  // After every render: React may have re-set the section's className.
  useLayoutEffect(() => {
    ref.current?.parentElement?.classList.toggle("collapsed", !open);
  });
  const H = level === 2 ? "h2" : "h3";
  return (
    <H
      ref={ref}
      className="with-help panel-title"
      // The title text folds too; buttons inside it (help, fold) keep their own click.
      onClick={(e) => {
        if (!(e.target as Element).closest("button")) setOpen(!open);
      }}
    >
      <button
        className="fold"
        aria-expanded={open}
        aria-label={open ? "Fold section" : "Unfold section"}
        title={open ? "Fold section" : "Unfold section"}
        onClick={() => setOpen(!open)}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M2 3.5l3 3 3-3"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      {children}
    </H>
  );
}
