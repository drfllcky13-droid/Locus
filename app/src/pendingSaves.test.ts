import { expect, test } from "vitest";
import { flushPending, whilePending } from "./pendingSaves";

test("closing waits for every save still pending, and only those", async () => {
  const saved: string[] = [];
  const later = (s: string) => () =>
    new Promise<void>((r) => setTimeout(() => (saved.push(s), r()), 5));
  const doneDiagram = whilePending(later("diagram"));
  whilePending(() => Promise.reject(new Error("disk full")));
  const doneScene = whilePending(later("scene"));
  doneScene(); // the scene saved on its own first
  await flushPending();
  expect(saved).toEqual(["diagram"]);
  doneDiagram();
});
