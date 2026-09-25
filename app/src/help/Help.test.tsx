// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { MeasureDialog } from "../diagram2d/MeasureDialog";

afterEach(cleanup);

test("the measured-point tool's ? opens the hand-measurements note", () => {
  render(<MeasureDialog points={[]} onPlace={() => {}} onClose={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "Help" }));
  expect(screen.getByRole("dialog", { name: "Method note" }).textContent).toContain(
    "Hand measurements",
  );
});
