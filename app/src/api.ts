// Typed wrappers over the Tauri commands in src-tauri/src/commands.rs and scene_cmds.rs.
// Types mirror the Rust crates' serde output.
import { Channel, invoke } from "@tauri-apps/api/core";
import type { PickHit, SceneData } from "./viewer3d/pointcloud";
import type { MeasurementRecord } from "./viewer3d/measureFormat";

export type LinearUnit = "meter" | "centimeter" | "millimeter" | "foot" | "us_survey_foot" | "inch";

export interface Bounds {
  min: [number, number, number];
  max: [number, number, number];
}

export interface ScanInfo {
  name: string;
  point_count: number;
  invalid_points: number;
  bounds: Bounds | null;
  pose: number[];
  attributes: string[];
}

export interface MeshInfo {
  name: string;
  vertex_count: number;
  face_count: number;
  bounds: Bounds | null;
}

export interface ExifField {
  ifd: string;
  tag: string;
  value: string;
}

export interface ImageInfo {
  name: string;
  kind: "photo" | "pinhole" | "spherical" | "cylindrical" | "visual_reference";
  width: number;
  height: number;
  scan: number | null;
  pose: number[] | null;
  exif: ExifField[];
}

export interface Contents {
  format: string;
  declared_unit: LinearUnit | null;
  crs: string | null;
  y_up: boolean;
  scans: ScanInfo[];
  meshes: MeshInfo[];
  images: ImageInfo[];
  warnings: string[];
}

export interface Preview {
  path: string;
  sha256: string;
  size: number;
  contents: Contents;
}

export interface EvidenceRecord {
  id: number;
  sha256: string;
  size: number;
  original_path: string;
  original_modified: string | null;
  stored_path: string;
  unit: LinearUnit | null;
  contents: Contents;
  imported_at: string;
  imported_by: string;
}

export interface ProjectInfo {
  name: string;
  root: string;
  examiner: string;
  evidence: EvidenceRecord[];
  audit_entries: number;
  audit_head: string;
  /** Latest evidence re-hash (on open or Verify Evidence); null for a new project. */
  integrity: IntegrityReport | null;
}

export interface IntegrityReport {
  results: [number, EvidenceStatus][];
  unrecorded: string[];
}

export interface Progress {
  stage: "reading" | "points" | "hashing" | "copying";
  done: number;
  total: number;
}

export type EvidenceStatus =
  { status: "intact" } | { status: "missing" } | { status: "changed"; actual: string };

function channel<T>(onMessage: (m: T) => void): Channel<T> {
  const c = new Channel<T>();
  c.onmessage = onMessage;
  return c;
}

export interface OctreeRecord {
  evidence_id: number;
  scan_idx: number;
  status: "building" | "built" | "failed";
  points: number;
  detail: string;
  updated_at: string;
}

export interface CleanupRecord {
  id: number;
  kind: "box_delete" | "lasso_delete" | "outliers" | "voxel";
  params: Record<string, unknown>;
  scans: { evidence_id: number; scan_idx: number; removed: number; file: string; sha256: string }[];
  active: boolean;
  created_at: string;
  created_by: string;
}

export interface StateView {
  revisions: Record<string, number>;
  octrees: OctreeRecord[];
  measurements: MeasurementRecord[];
  cleanups: CleanupRecord[];
  point_sigma_m: number;
}

export interface Resolved {
  scan: string;
  index: number;
  local: [number, number, number];
  project: [number, number, number];
}

export interface Region {
  min: [number, number, number];
  max: [number, number, number];
}

export type LassoDepth =
  | { mode: "all_depths" }
  | {
      mode: "visible_surface";
      viewport: [number, number];
      cell_px: number;
      tolerance_m: number;
      tolerance_rel: number;
    };

/** What the view was clipped to: [min, max, showInside] and [axis, offset, flip]. */
export interface LassoClip {
  clip_box: [[number, number, number], [number, number, number], boolean] | null;
  plane: [number, number, boolean] | null;
}

export type CleanupRequest =
  | { kind: "box_delete"; region: Region }
  | {
      kind: "lasso_delete";
      view_proj: number[];
      origin: [number, number, number];
      polygon: [number, number][];
      depth: LassoDepth;
      clip: LassoClip;
    }
  | { kind: "outliers"; k: number; std_mult: number; region: Region | null }
  | { kind: "voxel"; size: number; region: Region | null };

/** Settings for a registration run (src-tauri/src/register_cmds.rs `RunParams`). SI units. */
export interface RegistrationParams {
  sphere_radius: number | null;
  board_size: number | null;
  cloud: boolean;
  use_file_poses: boolean;
  cloud_sigma: number;
  target_tolerance: number;
  max_points: number;
  control: {
    name: string;
    kind: "sphere" | "board";
    position: [number, number, number];
    sigma: number;
  }[];
}

export type LinkKind = "Target" | "Cloud" | "Control";
export type LinkStatus = "Ok" | "Flagged" | "Untested";

export interface RegLink {
  kind: LinkKind;
  a: number;
  b: number | null;
  pairs: unknown[];
  forced: boolean;
  shape_only: boolean;
}

export interface RegLinkReport {
  status: LinkStatus;
  rms: number;
  max: number;
  chi2_per_dof: number;
  limit_per_dof: number;
}

export interface RegistrationRecord {
  id: number;
  parent: number | null;
  params: Record<string, unknown>;
  result: {
    scans: { evidence_id: number; scan_idx: number; name: string; points_used: number }[];
    links: RegLink[];
    reports: RegLinkReport[];
    verified: boolean[];
    iterations: number;
    summary: {
      links: number;
      ok: number;
      flagged: number;
      untested: number;
      shape_only: number;
      target_rms_mean_m: number | null;
      target_residual_max_m: number | null;
      unverified_scans: number;
    };
    extra: { overlap?: (number | null)[] };
  };
  poses: { evidence_id: number; scan_idx: number; pose: number[]; verified: boolean }[];
  applied: boolean;
  created_at: string;
  created_by: string;
}

export const api = {
  projectCreate: (parent: string, name: string, examinerName: string) =>
    invoke<ProjectInfo>("project_create", { parent, name, examinerName }),
  projectOpen: (root: string, examinerName: string, onProgress: (bytes: number) => void) =>
    invoke<ProjectInfo>("project_open", { root, examinerName, onProgress: channel(onProgress) }),
  importPreview: (path: string, onProgress: (p: Progress) => void) =>
    invoke<Preview>("import_preview", { path, onProgress: channel(onProgress) }),
  importCommit: (sha256: string, unit: LinearUnit | null, onProgress: (p: Progress) => void) =>
    invoke<{ project: ProjectInfo; evidence_id: number; warning: string | null }>("import_commit", {
      sha256,
      unit,
      onProgress: channel(onProgress),
    }),
  evidenceVerify: (onProgress: (bytes: number) => void) =>
    invoke<ProjectInfo>("evidence_verify", { onProgress: channel(onProgress) }),
  sceneView: () => invoke<SceneData>("scene_view"),
  analysisState: () => invoke<StateView>("analysis_state"),
  pickResolve: (pick: PickHit) => invoke<Resolved>("pick_resolve", { pick }),
  measure: (kind: MeasurementRecord["kind"], picks: PickHit[]) =>
    invoke<StateView>("measure", { kind, picks }),
  measurementDelete: (id: number) => invoke<StateView>("measurement_delete", { id }),
  setPointSigma: (meters: number) => invoke<StateView>("set_point_sigma", { meters }),
  cleanupApply: (request: CleanupRequest) => invoke<StateView>("cleanup_apply", { request }),
  cleanupPreview: (request: CleanupRequest) => invoke<number>("cleanup_preview", { request }),
  cleanupSetActive: (id: number, active: boolean) =>
    invoke<StateView>("cleanup_set_active", { id, active }),
  appInfo: () => invoke<{ version: string; webview: string }>("app_info"),
  startup: () => invoke<{ open: string | null; examiner: string | null }>("startup"),
  registrations: () => invoke<RegistrationRecord[]>("registrations"),
  registrationRun: (params: RegistrationParams) =>
    invoke<RegistrationRecord[]>("registration_run", { params }),
  registrationEdit: (id: number, del: number[], force: number[]) =>
    invoke<RegistrationRecord[]>("registration_edit", { id, delete: del, force }),
  registrationApply: (id: number | null) =>
    invoke<RegistrationRecord[]>("registration_apply", { id }),
};
