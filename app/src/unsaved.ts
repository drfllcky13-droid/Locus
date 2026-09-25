// Analysis work that exists only on screen until "Save analysis": picks, stains, pairs. A tool
// holding some registers it, so closing Lotus or switching projects asks before it is lost.
import { useEffect } from "react";
import { ask } from "@tauri-apps/plugin-dialog";

const held = new Set<string>();

/** Register `label` (e.g. "Trajectory") while `dirty`. */
export function useUnsaved(label: string, dirty: boolean) {
  useEffect(() => {
    if (!dirty) return;
    held.add(label);
    return () => void held.delete(label);
  }, [label, dirty]);
}

export const unsavedWork = (): string[] => [...held];

/** The question to ask, or null when nothing would be lost. */
export function discardQuestion(action: string, work = unsavedWork()): string | null {
  if (!work.length) return null;
  const what = work.length === 1 ? work[0] : `${work.slice(0, -1).join(", ")} and ${work.at(-1)}`;
  return `${what}: work not yet saved as an analysis will be lost. ${action} anyway?`;
}

/** True when nothing unsaved would be lost, or the examiner chooses to discard it. */
export async function confirmDiscard(action: string): Promise<boolean> {
  const q = discardQuestion(action);
  return (
    q === null ||
    ask(q, {
      title: "Unsaved analysis work",
      kind: "warning",
      okLabel: action,
      cancelLabel: "Go back",
    })
  );
}
