// The licence in force (src-tauri/src/license_cmds.rs), for showing only the tools it includes.
// The backend refuses the rest itself; this only keeps them out of the way.
import { createContext, useContext } from "react";

export type Tier = "diagram" | "analyst" | "analyst_plus";
export type Feature =
  | "diagrams"
  | "point_clouds"
  | "analysis"
  | "photogrammetry"
  | "animation"
  | "case_package"
  | "registration"
  | "crush_volume";

export interface LicenseInfo {
  tier: Tier;
  tier_name: string;
  status: "licensed" | "evaluation" | "invalid";
  license: {
    id: string;
    licensee: string;
    tier: Tier;
    issued: string;
    expires: string | null;
  } | null;
  problem: string | null;
}

const RANK: Record<Tier, number> = { diagram: 0, analyst: 1, analyst_plus: 2 };

export function allows(tier: Tier, f: Feature): boolean {
  if (f === "diagrams") return true;
  if (f === "registration" || f === "crush_volume") return RANK[tier] >= 2;
  return RANK[tier] >= 1;
}

export const License = createContext<LicenseInfo | null>(null);
/** Whether the licence includes `f` (everything until the licence has loaded). */
export const useAllows = (f: Feature) => {
  const l = useContext(License);
  return !l || allows(l.tier, f);
};
