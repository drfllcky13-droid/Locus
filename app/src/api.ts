// Typed wrappers over the Tauri commands in src-tauri/src/commands.rs.
// Types mirror locus-core's serde output.
import { Channel, invoke } from "@tauri-apps/api/core";

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

export const api = {
  projectCreate: (parent: string, name: string, examinerName: string) =>
    invoke<ProjectInfo>("project_create", { parent, name, examinerName }),
  projectOpen: (root: string, examinerName: string) =>
    invoke<ProjectInfo>("project_open", { root, examinerName }),
  importPreview: (path: string, onProgress: (p: Progress) => void) =>
    invoke<Preview>("import_preview", { path, onProgress: channel(onProgress) }),
  importCommit: (sha256: string, unit: LinearUnit | null, onProgress: (p: Progress) => void) =>
    invoke<{ project: ProjectInfo; evidence_id: number; warning: string | null }>("import_commit", {
      sha256,
      unit,
      onProgress: channel(onProgress),
    }),
  evidenceVerify: (onProgress: (bytes: number) => void) =>
    invoke<{ project: ProjectInfo; results: [number, EvidenceStatus][] }>("evidence_verify", {
      onProgress: channel(onProgress),
    }),
};
