// @vitest-environment jsdom
import { expect, test } from "vitest";
import { isViewKey } from "./keys";

function keyFrom(target: EventTarget): KeyboardEvent {
  let seen: KeyboardEvent | null = null;
  window.addEventListener("keydown", (e) => (seen = e), { once: true });
  target.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true }));
  return seen!;
}

test("a shortcut reaches the view that is shown, unless a field has focus", () => {
  document.body.innerHTML =
    "<div><input><select></select><textarea></textarea><canvas></canvas></div>";
  const canvas = document.querySelector("canvas")!;
  expect(isViewKey(keyFrom(canvas), true)).toBe(true);
  // The 3D view hidden behind a diagram tab must not undo its cleanups on Ctrl+Z.
  expect(isViewKey(keyFrom(canvas), false)).toBe(false);
  for (const field of ["input", "select", "textarea"])
    expect(isViewKey(keyFrom(document.querySelector(field)!), true)).toBe(false);
});
