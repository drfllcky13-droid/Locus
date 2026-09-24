// The guided workflows: each job's steps, what to do and why, where to do it, and how the
// project shows the step is done (from guide_state).
import type { Topic } from "../help/Help";

export interface GuideState {
  scans: number;
  photos: number;
  videos: number;
  verified: boolean;
  registrations: number;
  cleanups: number;
  measurements: number;
  diagrams: number;
  diagram_prints: number;
  scenes: number;
  animations: number;
  analyses: Record<string, number>;
  reports_printed: number;
  case_reports: number;
  packages: number;
}

export interface Step {
  title: string;
  /** What to do and why. */
  body: string;
  /** Where in the app. */
  where: string;
  help?: Topic;
  optional?: boolean;
  /** Done when this holds (undefined: the examiner ticks it). */
  done?: (s: GuideState, hasProject: boolean) => boolean;
}

export interface Guide {
  id: string;
  name: string;
  summary: string;
  steps: Step[];
}

const n = (s: GuideState, tool: string) => s.analyses[tool] ?? 0;

const START: Step[] = [
  {
    title: "Make a project for the case",
    body: "One project per case. Everything you import is kept read-only with its SHA-256, and every change is written to the project's audit log.",
    where: "File → New project…",
    help: "audit-log",
    done: (_, p) => p,
  },
  {
    title: "Import the scan",
    body: "Import each scan as evidence. A file that doesn't say its unit asks for one: check it against your scanner's export settings.",
    where: "Import evidence… (left panel)",
    done: (s) => s.scans > 0,
  },
];

const END: Step[] = [
  {
    title: "Print the reports",
    body: "Each analysis prints its method, inputs, results, uncertainty, assumptions and limitations. The case report lists every piece of evidence and record with its hash.",
    where: "Each tool's list: Report PDF…; 3D view → Export → Case report…",
    help: "exports",
    done: (s) => s.reports_printed > 0 && s.case_reports > 0,
  },
  {
    title: "Make a case package",
    body: "A read-only folder for a USB drive: the viewer, the case data, every report and render, all hashed. Give it to anyone who needs to see the case without Lotus.",
    where: "3D view → Export → Make a case package…",
    help: "case-package",
    done: (s) => s.packages > 0,
  },
];

export const GUIDES: Guide[] = [
  {
    id: "indoor",
    name: "Indoor crime scene",
    summary: "Scan, measure, diagram, bloodstain area of origin, reports and a case package.",
    steps: [
      ...START,
      {
        title: "Import the stain photos",
        body: "Close-up photos of each stain with a scale in view. They're evidence too, hashed like the scan.",
        where: "Import evidence… (choose the photos)",
        done: (s) => s.photos > 0,
      },
      {
        title: "Look around and clean up",
        body: "Orbit and zoom in the 3D view. Remove stray points (people, passing cars) with the cleanup tools. Cleanup never changes the evidence, and each operation can be undone.",
        where: "3D view → Cleanup",
        help: "cleanup",
        optional: true,
        done: (s) => s.cleanups > 0,
      },
      {
        title: "Measure",
        body: "Click two points for a distance. Each measurement has its uncertainty from the scan's point accuracy, and is saved with the exact points used.",
        where: "3D view → Tools → Distance",
        help: "measurement",
        done: (s) => s.measurements > 0,
      },
      {
        title: "Draw the room",
        body: "A diagram to scale: walls, doors, evidence markers, dimensions. It prints at a stated scale with a calibration bar.",
        where: "+ New diagram (tabs at the top)",
        help: "diagrams",
        done: (s) => s.diagrams > 0,
      },
      {
        title: "Find the area of origin",
        body: "Add each wall stain from its photo: line the photo up on the scan with two or more pairs, let Lotus find its edge, then mark its tail. At least four stains clearly moving upward are needed. The result is the point in 3D with its 95 % region, and the conventional point beside it.",
        where: "3D view → Bloodstain area of origin",
        help: "bloodstain",
        done: (s) => n(s, "bloodstain") > 0,
      },
      ...END,
    ],
  },
  {
    id: "crash",
    name: "Fatal crash",
    summary: "Scans of the road, speeds from marks and the EDR, crush, an animation, reports.",
    steps: [
      ...START,
      {
        title: "Register the scans",
        body: "Several scan stations are joined into one scene by their targets and overlapping surfaces. The registration report shows every link's error.",
        where: "Tools → Register scans (Analyst Plus)",
        help: "registration",
        optional: true,
        done: (s) => s.registrations > 0,
      },
      {
        title: "Measure the marks",
        body: "Tyre marks, rest positions and the area of impact, each measured on the scan with its uncertainty.",
        where: "3D view → Tools → Distance",
        help: "measurement",
        done: (s) => s.measurements > 0,
      },
      {
        title: "Speeds",
        body: "Speed from skid or yaw marks, momentum between the vehicles, and the EDR's pre-crash data. Each input has a range, and each result its range and interval.",
        where: "3D view → Crash reconstruction",
        help: "crash",
        done: (s) =>
          n(s, "skid") + n(s, "yaw") + n(s, "momentum") + n(s, "edr") + n(s, "crush") > 0,
      },
      {
        title: "Draw the scene",
        body: "A scaled diagram of the road: lanes, marks, rest positions, the area of impact.",
        where: "+ New diagram",
        help: "diagrams",
        done: (s) => s.diagrams > 0,
      },
      {
        title: "Animate the crash",
        body: "Vehicles on paths picked on the scan, moved by the EDR record or analysis results. Assumed parts are marked, and the checks flag anything the stated friction can't allow.",
        where: "3D view → 3D scene → Animation",
        help: "animation",
        done: (s) => s.animations > 0,
      },
      {
        title: "Time–distance–speed report and a render",
        body: "The table behind the animation, and an MP4 from a chosen view (a driver's view carries its field of view and limitation).",
        where: "3D view → Animation → report and Render to MP4",
        help: "animation",
        optional: true,
        done: (s) => n(s, "animation") > 0 || n(s, "render") > 0,
      },
      ...END,
    ],
  },
  {
    id: "fire",
    name: "Fire scene",
    summary:
      "Scans and photos of the scene, measurements, a diagram with the origin area marked, reports.",
    steps: [
      ...START,
      {
        title: "Import the photos or a video",
        body: "Photos and walkthrough video are evidence. From enough overlapping photos Lotus can also build a measured point cloud (photogrammetry).",
        where: "Import evidence…; 3D view → Photogrammetry",
        help: "photogrammetry",
        done: (s) => s.photos + s.videos > 0,
      },
      {
        title: "Clean up the scan",
        body: "Remove debris, people and equipment that aren't part of the scene. Nothing is deleted from the evidence.",
        where: "3D view → Cleanup",
        help: "cleanup",
        optional: true,
        done: (s) => s.cleanups > 0,
      },
      {
        title: "Measure burn patterns and heights",
        body: "Distances, heights above the floor and areas (for example a burn pattern's extent), each with its uncertainty.",
        where: "3D view → Tools → Distance, Height, Area",
        help: "measurement",
        done: (s) => s.measurements > 0,
      },
      {
        title: "Draw the scene",
        body: "Rooms, openings, fuel packages and the area of origin as you've determined it, with evidence markers and dimensions. Lotus documents and measures; the origin and cause are your determination.",
        where: "+ New diagram",
        help: "diagrams",
        done: (s) => s.diagrams > 0,
      },
      ...END,
    ],
  },
];
