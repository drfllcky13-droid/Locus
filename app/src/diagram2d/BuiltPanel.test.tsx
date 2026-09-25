// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { ItemPanel } from "./BuiltPanel";

afterEach(cleanup);

test("a north arrow turns by degrees typed in, stored in radians", () => {
  const onChange = vi.fn();
  render(
    <ItemPanel
      entity={{ id: "n", layer: "base", kind: "north", at: [0, 0], rotation: 0 }}
      onChange={onChange}
    />,
  );
  fireEvent.change(screen.getByLabelText("North, anticlockwise from up (°)"), {
    target: { value: "30" },
  });
  expect(onChange.mock.calls[0][0].rotation).toBeCloseTo(Math.PI / 6, 12);
});

test("a symbol's scale and a text's words and height can be changed", () => {
  const onChange = vi.fn();
  render(
    <ItemPanel
      entity={{
        id: "s",
        layer: "base",
        kind: "symbol",
        symbol: "car",
        at: [0, 0],
        rotation: Math.PI / 2,
        scale: 1,
      }}
      onChange={onChange}
    />,
  );
  expect(screen.getByLabelText<HTMLInputElement>("Rotation, anticlockwise (°)").value).toBe("90");
  fireEvent.change(screen.getByLabelText("Scale (×)"), { target: { value: "1.5" } });
  expect(onChange.mock.calls[0][0].scale).toBe(1.5);
  cleanup();
  onChange.mockClear();
  render(
    <ItemPanel
      entity={{
        id: "t",
        layer: "base",
        kind: "text",
        at: [0, 0],
        text: "Kitchen",
        height: 3,
        rotation: 0,
      }}
      onChange={onChange}
    />,
  );
  fireEvent.change(screen.getByLabelText("Text"), { target: { value: "Hall" } });
  fireEvent.change(screen.getByLabelText("Height on paper (mm)"), { target: { value: "5" } });
  expect(onChange.mock.calls.map((c) => [c[0].text, c[0].height])).toEqual([
    ["Hall", 3],
    ["Kitchen", 5],
  ]);
});
