import { describe, expect, it } from "vitest";
import type { LinkKind, LinkStatus, RegistrationRecord } from "../api";
import { edgeStyle, layout } from "./graph";

const pose = (x: number, y: number) => [1, 0, 0, x, 0, 1, 0, y, 0, 0, 1, 0, 0, 0, 0, 1];
const link = (a: number, b: number | null, kind: LinkKind = "Cloud", shape_only = false) => ({
  kind,
  a,
  b,
  pairs: [],
  forced: false,
  shape_only,
});
const report = (status: LinkStatus) => ({
  status,
  rms: 0.001,
  max: 0.002,
  chi2_per_dof: 1,
  limit_per_dof: 2,
});

function reg(): RegistrationRecord {
  return {
    id: 1,
    parent: null,
    params: {},
    result: {
      scans: [0, 1, 2].map((i) => ({ evidence_id: 1, scan_idx: i, name: `S${i}`, points_used: 1 })),
      links: [
        link(0, 1, "Target"),
        link(0, 1),
        link(1, 2, "Cloud", true),
        link(0, null, "Control"),
      ],
      reports: [report("Ok"), report("Flagged"), report("Ok"), report("Untested")],
      verified: [true, true, false],
      iterations: 3,
      summary: {
        links: 4,
        ok: 2,
        flagged: 1,
        untested: 1,
        shape_only: 1,
        target_rms_mean_m: null,
        target_residual_max_m: null,
        unverified_scans: 1,
      },
      extra: {},
    },
    poses: [pose(0, 0), pose(10, 0), pose(10, 5)].map((p, i) => ({
      evidence_id: 1,
      scan_idx: i,
      pose: p,
      verified: i < 2,
    })),
    applied: false,
    created_at: "",
    created_by: "",
  };
}

describe("registration graph", () => {
  it("places scans at their registered positions, to a uniform scale, y up", () => {
    const { nodes } = layout(reg(), 200, 20);
    // Span 10 m across 160 px: 16 px per metre, centred.
    expect(nodes[0]).toMatchObject({ x: 20, y: 140 });
    expect(nodes[1]).toMatchObject({ x: 180, y: 140 });
    expect(nodes[2]).toMatchObject({ x: 180, y: 60 });
    expect(nodes.map((n) => n.verified)).toEqual([true, true, false]);
  });

  it("colours links by status, shape-only and queued deletion; fans out parallel links", () => {
    const { edges } = layout(reg(), 200, 20, new Set([3]));
    expect(edges.map((e) => e.style)).toEqual(["ok", "flagged", "shape-only", "deleted"]);
    expect(edges.map((e) => e.lane)).toEqual([0, 1, 0, 0]);
    expect(edges[3].to).toBeNull();
  });

  it("a flagged shape-only link reads as flagged", () => {
    expect(edgeStyle(link(0, 1, "Cloud", true), report("Flagged"))).toBe("flagged");
    expect(edgeStyle(link(0, 1), report("Untested"))).toBe("untested");
  });
});
