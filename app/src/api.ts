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
  bloodstainAlign: (request: StainRequest["align"]) =>
    invoke<Alignment>("bloodstain_align", { request }),
  /** Edge points of the dark region around `seed` in a greyscale crop (crop pixels). */
  bloodstainEdges: (
    luma: number[],
    width: number,
    height: number,
    seed: [number, number],
    threshold: number,
  ) => invoke<[number, number][]>("bloodstain_edges", { luma, width, height, seed, threshold }),
  bloodstainStain: (request: StainRequest, parameters: BloodstainParameters) =>
    invoke<{ input: StainInput; result: StainResult }>("bloodstain_stain", {
      request,
      parameters,
    }),
  bloodstainPreview: (request: BloodstainRequest) =>
    invoke<BloodstainRun>("bloodstain_preview", { request }),
  bloodstainSave: (name: string, request: BloodstainRequest, revises: number | null) =>
    invoke<AnalysisRecord>("bloodstain_save", { name, request, revises }),
  cameraPreview: (request: CameraRequest) => invoke<CameraRun>("camera_preview", { request }),
  cameraSave: (name: string, request: CameraRequest, revises: number | null) =>
    invoke<AnalysisRecord>("camera_save", { name, request, revises }),
  witnessPreview: (request: WitnessRequest) => invoke<WitnessRun>("witness_preview", { request }),
  witnessSave: (name: string, request: WitnessRequest, revises: number | null) =>
    invoke<AnalysisRecord>("witness_save", { name, request, revises }),
  analyses: () => invoke<AnalysisRecord[]>("analyses"),
  analysisWithdraw: (id: number, reason: string) =>
    invoke<AnalysisRecord[]>("analysis_withdraw", { id, reason }),
  caseNumber: () => invoke<string | null>("case_number"),
  caseNumberSet: (value: string) => invoke<void>("case_number_set", { value }),
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
  conventions: {
    /** "level_perpendicular" or "normal". */
    surface: string;
    reference: string;
    /** The reference axis, degrees clockwise from project +y. */
    reference_deg: number;
  };
}

export interface TrajectoryRequest {
  points: {
    pick: PickHit;
    kind: "entry" | "exit" | "rod";
    surface: string;
    /** 1σ for a rod point or a manual centre (a fitted centre carries its own). */
    sigma: number;
    centre: "fitted" | "manual";
    override_reason: string | null;
    /** Evidence id of a photo of the defect. */
    photo: number | null;
  }[];
  parameters: TrajectoryParameters;
  plane_radius: number;
  hole_radius: number;
}

export interface Band {
  centre: [[number, number], [P3, P3]] | null;
  footprint: [number, number][];
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
    centre?: string;
    override_reason?: string | null;
    defect?: {
      centre: P3;
      centre_sigma: number;
      semi_axes: [Measured, Measured];
      impact: Measured;
      rim_points: number;
      spacing: number;
    } | null;
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
    level?: { vertical: Measured; horizontal: Measured | null } | null;
  }[];
  band: Band;
  band_measurement?: Band | null;
  scene_bearing?: Measured | null;
  cross_checks?: {
    input: number;
    surface: string;
    kind: string;
    ellipse: Measured;
    trajectory: Measured;
    difference: number;
    agrees: boolean;
  }[];
  cone_narrower_than_fit: boolean;
  summary: string;
  assumptions: string[];
  limitations: string[];
}

interface AnalysisBase {
  id: number;
  method: string;
  name: string;
  sha256: string;
  revises: number | null;
  created_at: string;
  created_by: string;
  withdrawn: { at: string; by: string; reason: string } | null;
}

/** A stored analysis run; `record` is the tool's run type. */
export type AnalysisRecord = AnalysisBase &
  (
    | { tool: "trajectory"; record: TrajectoryRun }
    | { tool: "bloodstain"; record: BloodstainRun }
    | { tool: "camera"; record: CameraRun }
    | { tool: "witness"; record: WitnessRun }
  );
export type TrajectoryRecord = Extract<AnalysisRecord, { tool: "trajectory" }>;
export type BloodstainRecord = Extract<AnalysisRecord, { tool: "bloodstain" }>;
export type CameraRecord = Extract<AnalysisRecord, { tool: "camera" }>;
export type WitnessRecord = Extract<AnalysisRecord, { tool: "witness" }>;

export type LensModel = "pinhole" | "radial1" | "radial2" | "full";

/** A solved camera (locus-analysis camera::Camera): rows of `rotation` are its right, down
 * and forward axes in the project frame. */
export interface SolvedCamera {
  position: P3;
  rotation: [P3, P3, P3];
  size: [number, number];
  f: number;
  cx: number;
  cy: number;
  /** k1, k2, k3, p1, p2. */
  distortion: [number, number, number, number, number];
}

export interface CameraParameters {
  model: LensModel;
  pick_sigma_px: number;
  point_sigma: number;
  floor_z: number;
  draws: number;
  seed: number;
}

export interface HeightInput {
  label: string;
  feet_px: [number, number];
  head_px: [number, number];
  matched_model: number | null;
}

export interface CameraRequest {
  photo: number;
  size: [number, number];
  pairs: { px: [number, number]; pick: PickHit }[];
  parameters: CameraParameters;
  subjects: HeightInput[];
}

export interface CameraRun {
  method: string;
  photo: { evidence_id: number; name: string; file: string; sha256: string } | null;
  pairs: { px: [number, number]; world: P3 }[];
  parameters: CameraParameters;
  solve: {
    model: LensModel;
    camera: SolvedCamera;
    position_sigma: P3;
    angles_sigma: P3;
    f_sigma: number;
    principal_sigma: [number, number];
    residuals: [number, number][];
    residual_sigmas: number[];
    rms_px: number;
    chi2: number;
    dof: number;
    birge: number;
    warnings: string[];
  };
  heights: {
    input: HeightInput;
    height: Measured;
    interval95: [number, number];
    feet: P3;
    head: P3;
    miss: number;
  }[];
  summary: string;
}

export interface WitnessRequest {
  floor: PickHit;
  eye_height: number;
  look_at: P3;
  fov_deg: number;
  targets: { label: string; pick: PickHit }[];
  radius?: number;
  end_clearance?: number;
}

export interface WitnessRun {
  method: string;
  floor_point: P3;
  eye_height: number;
  eye: P3;
  look_at: P3;
  fov_deg: number;
  sights: {
    label: string;
    from: P3;
    to: P3;
    length: number;
    clear: boolean;
    blocking: number;
    first: P3 | null;
    first_distance: number | null;
  }[];
  summary: string;
}

type P2 = [number, number];

export interface BloodstainParameters {
  floor_z: number;
  bootstrap: number;
  seed: number;
  /** The examiner's reason for using stains not clearly moving upward. */
  include_not_upward: string | null;
  reference: string;
  reference_deg: number;
  /** Also find where floor stains converge in plan (a separate 2-D result). */
  floor_convergence: boolean;
}

/** A perspective correction from a scale's four corners (row-major pixel → rectified pixel). */
export interface Rectification {
  corners_px: [P2, P2, P2, P2];
  /** Width and height (m). */
  size: P2;
  corner_sigma_px: number;
  h: [P3, P3, P3];
  pixels_per_metre: number;
  stretch: number;
}

/** A photo placed on its surface: pixel p is at origin + q.x·x_step + q.y·y_step, for q the
 * pixel after the perspective correction (p itself without one). */
export interface Alignment {
  pairs: { px: P2; world: P3 }[];
  plane_point: P3;
  plane_normal: P3;
  origin: P3;
  x_step: P3;
  y_step: P3;
  pixels_per_metre: number;
  residuals: number[];
  rms: number | null;
  /** 1σ of the photo's rotation on the surface (degrees). */
  rotation_sigma_deg: number;
  rectification: Rectification | null;
  scale_ratio: number | null;
  plane_rms: number;
}

/** Four corners of a rectangle on the scale, in order around it, and its size (m). */
export interface ScaleCorners {
  corners_px: [P2, P2, P2, P2];
  size: P2;
  corner_sigma_px?: number | null;
}

export interface StainRequest {
  label: string;
  surface: string;
  /** Evidence id of the stain's photo. */
  photo: number;
  align: {
    pairs: { px: P2; pick: PickHit }[];
    eye: P3;
    plane_radius?: number;
    scale?: ScaleCorners | null;
  };
  edges: P2[];
  auto_edge: { seed: P2; threshold: number } | null;
  tail_px: P2;
  excluded: string | null;
}

export interface BloodstainRequest {
  stains: StainRequest[];
  parameters: BloodstainParameters;
}

export interface StainInput {
  label: string;
  surface: string;
  centre: P3;
  normal: P3;
  width: Measured;
  length: Measured;
  travel: P3;
  travel_sigma_deg: number;
  excluded: string | null;
  fit: {
    edge_points: number;
    trimmed: number;
    rms: number;
    axis_sigma_deg: number;
    perspective: { stretch: number; width_sigma: number } | null;
  } | null;
  alignment: Alignment | null;
}

export interface StainResult {
  impact: Measured;
  directionality: Measured;
  ray: P3;
  upward: boolean;
  clearly_upward: boolean;
  used: boolean;
  not_used: string | null;
  residual: number;
  residual_sigmas: number;
  behind: boolean;
  /** Share of the origin fit's information (0 for a stain not used). */
  influence: number;
}

/** A stored or previewed bloodstain run (locus-analysis bloodstain::Run). */
export interface BloodstainRun {
  method: string;
  inputs: StainInput[];
  parameters: BloodstainParameters;
  stains: StainResult[];
  origin: {
    point: P3;
    height: Measured;
    sigma: P3;
    ellipsoid: { semi_axes: P3; axes: [P3, P3, P3] };
    rms_residual: number;
    chi2: number;
    dof: number;
    stains_used: number;
    bootstrap: number;
    bootstrap_failed: number;
    conditioning: number;
    near_round_share: number;
  };
  /** The conventional point (perpendicular distances), for comparison. */
  conventional: {
    point: P3;
    height: Measured;
    ellipsoid: { semi_axes: P3; axes: [P3, P3, P3] };
    shift: number;
  } | null;
  /** Floor stains' plan-view convergence: a separate 2-D result. */
  convergence: {
    point: P2;
    sigma: P2;
    semi_axes: P2;
    axis: P2;
    conventional: P2;
    chi2: number;
    dof: number;
    stains: number[];
  } | null;
  convergence_note: string | null;
  summary: string;
  assumptions: string[];
  limitations: string[];
}
