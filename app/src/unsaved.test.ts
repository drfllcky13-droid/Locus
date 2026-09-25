import { describe, expect, it } from "vitest";
import { discardQuestion } from "./unsaved";

describe("unsaved analysis work", () => {
  it("asks only when something would be lost, naming each tool", () => {
    expect(discardQuestion("Close Lotus", [])).toBeNull();
    expect(discardQuestion("Close Lotus", ["Trajectory"])).toBe(
      "Trajectory: work not yet saved as an analysis will be lost. Close Lotus anyway?",
    );
    expect(discardQuestion("Open project", ["Trajectory", "Camera match", "Witness view"])).toMatch(
      /^Trajectory, Camera match and Witness view: /,
    );
  });
});
