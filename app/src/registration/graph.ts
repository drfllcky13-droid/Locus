// Layout and colouring of the registration graph: scans drawn where they were registered,
// seen from above, links between them coloured by what the pose graph concluded.
import type { RegistrationRecord, RegLink, RegLinkReport } from "../api";

export interface GraphNode {
  scan: number;
  name: string;
  x: number;
  y: number;
  verified: boolean;
}

export interface GraphEdge {
  link: number;
  from: number;
  /** null for a control link (drawn as a stub). */
  to: number | null;
  /** Several links between the same scans are fanned out by this index. */
  lane: number;
  style: EdgeStyle;
}

export type EdgeStyle = "ok" | "flagged" | "untested" | "shape-only" | "deleted";

/** How a link is drawn. A queued deletion wins, then flagged, then shape-only. */
export function edgeStyle(link: RegLink, report: RegLinkReport, queuedDelete = false): EdgeStyle {
  if (queuedDelete) return "deleted";
  if (report.status === "Flagged") return "flagged";
  if (link.shape_only) return "shape-only";
  if (report.status === "Untested") return "untested";
  return "ok";
}

/**
 * Node positions in a `size` × `size` box with `margin`, from each scan's registered
 * position (x, y of its pose translation; y up on screen). The scale is uniform, so
 * distances on screen are proportional to real ones.
 */
export function layout(
  reg: RegistrationRecord,
  size: number,
  margin: number,
  queuedDelete: Set<number> = new Set(),
): { nodes: GraphNode[]; edges: GraphEdge[] } {
  const pos = reg.result.scans.map((s) => {
    const p = reg.poses.find((q) => q.evidence_id === s.evidence_id && q.scan_idx === s.scan_idx);
    return p ? [p.pose[3], p.pose[7]] : [0, 0];
  });
  const xs = pos.map((p) => p[0]);
  const ys = pos.map((p) => p[1]);
  const [minX, maxX] = [Math.min(...xs), Math.max(...xs)];
  const [minY, maxY] = [Math.min(...ys), Math.max(...ys)];
  const span = Math.max(maxX - minX, maxY - minY, 1e-9);
  const k = (size - 2 * margin) / span;
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const nodes = reg.result.scans.map((s, i) => ({
    scan: i,
    name: s.name,
    x: size / 2 + (pos[i][0] - cx) * k,
    y: size / 2 - (pos[i][1] - cy) * k,
    verified: reg.result.verified[i] ?? false,
  }));
  const seen = new Map<string, number>();
  const edges = reg.result.links.map((l, i) => {
    const key = l.b === null ? `c${l.a}` : `${Math.min(l.a, l.b)}-${Math.max(l.a, l.b)}`;
    const lane = seen.get(key) ?? 0;
    seen.set(key, lane + 1);
    return {
      link: i,
      from: l.a,
      to: l.b,
      lane,
      style: edgeStyle(l, reg.result.reports[i], queuedDelete.has(i)),
    };
  });
  return { nodes, edges };
}
