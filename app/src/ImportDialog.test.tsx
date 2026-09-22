// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, test, vi } from "vitest";
import type { Contents, Preview } from "./api";
import { ImportDialog } from "./ImportDialog";

afterEach(() => {
  cleanup();
  clearMocks();
});

const SHA = "3f9a1c2b4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8";

function preview(contents: Partial<Contents>): Preview {
  return {
    path: "C:/cases/scene.e57",
    sha256: SHA,
    size: 1536,
    contents: {
      format: "E57",
      declared_unit: "meter",
      crs: null,
      y_up: false,
      scans: [],
      meshes: [],
      images: [],
      warnings: [],
      ...contents,
    },
  };
}

const scan = (name: string, point_count: number) => ({
  name,
  point_count,
  invalid_points: 0,
  bounds: {
    min: [0, 0, 0] as [number, number, number],
    max: [2, 3, 4] as [number, number, number],
  },
  pose: [],
  attributes: [],
});

test("shows the file hash and point count before import", async () => {
  mockIPC((cmd) =>
    cmd === "import_preview" ? preview({ scans: [scan("A", 10), scan("B", 3)] }) : null,
  );
  render(<ImportDialog path="C:/cases/scene.e57" onClose={() => {}} onImported={() => {}} />);

  expect(await screen.findByText(SHA)).toBeTruthy();
  expect(screen.getByText("13 points in 2 scans")).toBeTruthy();
  expect(screen.getByText("Meters (stated by the file)")).toBeTruthy();
  expect(screen.getAllByText("2.000 m × 3.000 m × 4.000 m")).toHaveLength(2);
});

test("a file without a unit cannot be imported until the examiner picks one", async () => {
  const calls: Record<string, unknown>[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "import_preview")
      return preview({ format: "XYZ", declared_unit: null, scans: [scan("pts", 3)] });
    if (cmd === "import_commit") {
      calls.push(args as Record<string, unknown>);
      return { project: {}, evidence_id: 1, warning: null };
    }
  });
  const onImported = vi.fn();
  render(<ImportDialog path="C:/cases/pts.xyz" onClose={() => {}} onImported={onImported} />);

  const button = (await screen.findByText("Import as evidence")) as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  fireEvent.change(screen.getByLabelText("Unit of the file's coordinates"), {
    target: { value: "us_survey_foot" },
  });
  expect(button.disabled).toBe(false);
  expect(screen.getAllByText("2.000 US ft × 3.000 US ft × 4.000 US ft")).toHaveLength(1);

  fireEvent.click(button);
  await vi.waitFor(() => expect(onImported).toHaveBeenCalled());
  expect(calls[0]).toMatchObject({ sha256: SHA, unit: "us_survey_foot" });
});

test("reader errors are shown, not swallowed", async () => {
  mockIPC((cmd) => {
    if (cmd === "import_preview")
      throw "could not read E57 file: the file's contents don't match its extension";
  });
  render(<ImportDialog path="C:/cases/fake.e57" onClose={() => {}} onImported={() => {}} />);
  expect(await screen.findByText(/contents don't match its extension/)).toBeTruthy();
});
