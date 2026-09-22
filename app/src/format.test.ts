import { expect, test } from "vitest";
import type { Contents } from "./api";
import { formatBytes, formatCount, formatExtent, needsUnit, progressPercent } from "./format";

test("bytes show a unit and the exact count", () => {
  expect(formatBytes(512)).toBe("512 B");
  expect(formatBytes(1536)).toBe("1.50 KiB (1,536 bytes)");
  expect(formatBytes(3 * 1024 ** 3)).toBe("3.00 GiB (3,221,225,472 bytes)");
});

test("counts pluralize", () => {
  expect(formatCount(1, "point")).toBe("1 point");
  expect(formatCount(100_000_000, "point")).toBe("100,000,000 points");
  expect(formatCount(2, "vertex", "vertices")).toBe("2 vertices");
});

test("extent is per-axis in the source unit", () => {
  expect(formatExtent({ min: [0, -1, 2], max: [1.5, 1, 2] }, "us_survey_foot")).toBe(
    "1.500 US ft × 2.000 US ft × 0.000 US ft",
  );
  expect(formatExtent(null, "meter")).toBe("no valid points");
});

const empty: Contents = {
  format: "XYZ",
  declared_unit: null,
  crs: null,
  y_up: false,
  scans: [],
  meshes: [],
  images: [],
  warnings: [],
};

test("unit is required only for geometry without a declared unit", () => {
  const scan = {
    name: "s",
    point_count: 1,
    invalid_points: 0,
    bounds: null,
    pose: [],
    attributes: [],
  };
  expect(needsUnit(empty)).toBe(false);
  expect(needsUnit({ ...empty, scans: [scan] })).toBe(true);
  expect(needsUnit({ ...empty, scans: [scan], declared_unit: "meter" })).toBe(false);
});

test("progress never exceeds 100", () => {
  expect(progressPercent(5, 0)).toBe(0);
  expect(progressPercent(50, 200)).toBe(25);
  expect(progressPercent(300, 200)).toBe(100);
});
