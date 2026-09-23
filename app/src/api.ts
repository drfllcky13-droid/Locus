// Typed wrappers over the Tauri commands in src-tauri/src/commands.rs and scene_cmds.rs.
// Types mirror the Rust crates' serde output.
import { Channel, invoke } from "@tauri-apps/api/core";
import type { PickHit, SceneData } from "./viewer3d/pointcloud";
import type { MeasurementRecord } from "./viewer3d/measureFormat";
import type { Diagram } from "./diagram2d/model";
import type { SceneDoc } from "./scene3d/model";

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

/** A stored document's revision (a diagram, or a 3D scene). */
export interface Revision<D> {
  document_id: number;
  revision_id: number;
  number: number;
  name: string;
  document: D;
  sha256: string;
  created_at: string;
  created_by: string;
}

export type DiagramRevision = Revision<Diagram>;
export type SceneRevision = Revision<SceneDoc>;

export type HandRequest =
  | {
      method: "baseline_offset";
      from: [number, number];
      to: [number, number];
      along: number;
      offset: number;
      side: "Left" | "Right";
    }
  | {
      method: "triangulation";
      refs: [[number, number], number][];
      side: "Left" | "Right" | null;
    };

export interface HandSolved {
  position: [number, number];
  covariance: [[number, number], [number, number]];
  residuals: number[];
  worst_normalised: number | null;
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
  thirdPartyNotices: () => invoke<string>("third_party_notices"),
  diagrams: () => invoke<DiagramRevision[]>("diagrams"),
  diagramCreate: (name: string, document: Diagram) =>
    invoke<DiagramRevision>("diagram_create", { name, document }),
  diagramSave: (diagramId: number, name: string, document: Diagram) =>
    invoke<DiagramRevision>("diagram_save", { diagramId, name, document }),
  /** Prints the newest saved revision; returns the PDF's SHA-256 (also logged). */
  diagramPdf: (
    diagramId: number,
    scale: number,
    paper: "A4" | "A3",
    landscape: boolean,
    path: string,
  ) => invoke<string>("diagram_pdf", { diagramId, scale, paper, landscape, path }),
  underlayImages: () =>
    invoke<
      {
        evidence_id: number;
        name: string;
        file: string;
        sha256: string;
        width: number;
        height: number;
      }[]
    >("underlay_images"),
  /** The image bytes, after the backend has checked their hash. */
  underlayBytes: (file: string, sha256: string) =>
    invoke<ArrayBuffer>("underlay_bytes", { file, sha256 }),
  underlaySlice: (zMin: number, zMax: number, resolution: number) =>
    invoke<{
      file: string;
      sha256: string;
      origin: [number, number];
      resolution: number;
      width: number;
      height: number;
      points: number;
    }>("underlay_slice", { zMin, zMax, resolution }),
  underlayCalibrated: (details: unknown) => invoke<void>("underlay_calibrated", { details }),
  scenes: () => invoke<SceneRevision[]>("scenes"),
  sceneCreate: (name: string, document: SceneDoc) =>
    invoke<SceneRevision>("scene_create", { name, document }),
  sceneSave: (sceneId: number, name: string, document: SceneDoc) =>
    invoke<SceneRevision>("scene_save", { sceneId, name, document }),
  diagramRevision: (revisionId: number) =>
    invoke<DiagramRevision>("diagram_revision", { revisionId }),
  /** The surface around a picked point (plane fit), its normal facing `toward`. */
  surfaceAt: (pick: PickHit, radius: number, toward: [number, number, number]) =>
    invoke<{
      point: [number, number, number];
      normal: [number, number, number];
      rms: number;
      max_abs: number;
      points: number;
    }>("surface_at", { pick, radius, toward }),
  sunPosition: (lat: number, lon: number, unix: number) =>
    invoke<{
      azimuth: number;
      elevation: number;
      apparent_elevation: number;
      declination: number;
      equation_of_time: number;
      uncertainty: number;
    }>("sun_position", { lat, lon, unix }),
  trajectoryPreview: (request: TrajectoryRequest) =>
    invoke<TrajectoryRun>("trajectory_preview", { request }),
  trajectorySave: (name: string, request: TrajectoryRequest, revises: number | null) =>
    invoke<AnalysisRecord>("trajectory_save", { name, request, revises }),
  analyses: () => invoke<AnalysisRecord[]>("analyses"),
  analysisWithdraw: (id: number, reason: string) =>
    invoke<AnalysisRecord[]>("analysis_withdraw", { id, reason }),
  /** Writes the PDF (from the stored record) and returns its SHA-256 (also logged). */
  analysisReport: (id: number, path: string) => invoke<string>("analysis_report", { id, path }),
  handSolve: (request: HandRequest, knownSigma: number, tapeFixed: number, tapePerMetre: number) =>
    invoke<HandSolved>("hand_solve", { request, knownSigma, tapeFixed, tapePerMetre }),
  startup: () => invoke<{ open: string | null; examiner: string | null }>("startup"),
  registrations: () => invoke<RegistrationRecord[]>("registrations"),
  registrationRun: (params: RegistrationParams) =>
    invoke<RegistrationRecord[]>("registration_run", { params }),
  registrationEdit: (id: number, del: number[], force: number[]) =>
    invoke<RegistrationRecord[]>("registration_edit", { id, delete: del, force }),
  registrationApply: (id: number | null) =>
    invoke<RegistrationRecord[]>("registration_apply", { id }),
  /** Writes the PDF and returns its SHA-256 (also recorded in the audit log). */
  registrationReport: (id: number, path: string) =>
    invoke<string>("registration_report", { id, path }),
};

/** A value with its 1σ uncertainty. */
export interface Measured {
  value: number;
  sigma: number;
}

export interface TrajectoryParameters {
  cone_deg: number;
  rod_play_deg: number;
  band: [number, number];
  floor_z: number;
  max_range: number;
}

export interface TrajectoryRequest {
  points: { pick: PickHit; kind: "entry" | "exit" | "rod"; surface: string; sigma: number }[];
  parameters: TrajectoryParameters;
  plane_radius: number;
}

type P3 = [number, number, number];

/** A stored or previewed trajectory run (locus-analysis trajectory::Run). */
export interface TrajectoryRun {
  method: string;
  inputs: {
    kind: string;
    surface: string;
    point: P3;
    sigma: number;
    plane: { point: P3; normal: P3; rms: number; points: number } | null;
  }[];
  parameters: TrajectoryParameters;
  line: {
    point: P3;
    direction: P3;
    bearing: Measured;
    elevation: Measured;
    cone: { major_deg: number; minor_deg: number; major_axis: P3 };
    residuals: number[];
    chi2: number;
    dof: number;
    inflation: number;
  };
  surfaces: {
    surface: string;
    angles: { impact: Measured; horizontal: Measured; vertical: Measured };
    plane_rms: number;
  }[];
  band: { centre: [[number, number], [P3, P3]] | null; footprint: [number, number][] };
  cone_narrower_than_fit: boolean;
  summary: string;
  assumptions: string[];
  limitations: string[];
}

export interface AnalysisRecord {
  id: number;
  tool: string;
  method: string;
  name: string;
  record: TrajectoryRun;
  sha256: string;
  revises: number | null;
  created_at: string;
  created_by: string;
  withdrawn: { at: string; by: string; reason: string } | null;
}
