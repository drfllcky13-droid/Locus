// Editors save a moment after edits settle. Each one holding edits not yet saved registers
// how to save them now, so closing the window first waits for them (App); otherwise the last
// edits before closing were lost.

const pending = new Set<() => Promise<unknown>>();

/** Register `save` until the returned function is called. */
export function whilePending(save: () => Promise<unknown>): () => void {
  pending.add(save);
  return () => void pending.delete(save);
}

/** Run every registered save and wait for all of them (failures included). */
export async function flushPending(): Promise<void> {
  await Promise.allSettled([...pending].map((save) => save()));
}
