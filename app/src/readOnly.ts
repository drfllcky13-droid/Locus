// A case package opens read-only: this tells every panel to show, not edit.
import { createContext, useContext } from "react";

export const ReadOnly = createContext(false);
export const useReadOnly = () => useContext(ReadOnly);

/** What the viewer found on opening a case package (locus-core package::PackageCheck). */
export interface PackageInfo {
  folder: string;
  check: {
    hash: string;
    checked: number;
    problems: string[];
    manifest: {
      project: string;
      case_number: string | null;
      made_by: string;
      made_at: string;
      app_version: string;
      source_state_head: string;
      evidence_included: boolean;
      not_included: string[];
      files: { path: string; sha256: string; bytes: number }[];
    };
  };
}
