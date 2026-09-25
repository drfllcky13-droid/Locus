// The 3D scene builder, in the 3D view's side panel: extrude diagrams, add roofs, models and
// lights, set the sun, and snap models to the point cloud. The document is saved as a new
// revision (audit-logged) once edits settle, like a diagram.
import { HelpButton } from "../help/Help";
import { PanelHeader } from "../PanelHeader";
import { useCallback, useEffect, useMemo, useState } from "react";
import * as THREE from "three";
import { api, type DiagramRevision, type SceneRevision } from "../api";
import type { Diagram } from "../diagram2d/model";
import type { Engine } from "../viewer3d/engine";
import type { PickHit } from "../viewer3d/pointcloud";
import { DEFAULT_EXTRUDE, type Part } from "./extrude";
import {
  FURNITURE,
  POSES,
  VEHICLES,
  vehicleProblem,
  vehicleSpec,
  WEAPONS,
  type Asset,
  type FurnitureItem,
  type Slot,
  type VehicleClass,
  type WeaponItem,
} from "./library";
import {
  EMPTY_SCENE,
  PRESETS,
  preset,
  placeMatrix,
  scaleOf,
  type DiagramRef,
  type MaterialDef,
  type SceneDoc,
  type SceneObject,
} from "./model";
import { buildScene, type Diagrams, type Meshes } from "./render";
import { basePoint, isMesh, loadEvidenceMesh } from "./evidenceMesh";
import { DEFAULT_ROOF, rectangle, type RoofType } from "./roof";
import { walls } from "../diagram2d/builders";
import { AnimationPanel } from "../animation/AnimationPanel";
import { useReadOnly } from "../readOnly";
import { savePath } from "../export/ExportPanel";
import { sceneGlb } from "./gltf";
import { whilePending } from "../pendingSaves";
import type { EvidenceRecord } from "../api";

const AUTOSAVE_MS = 1500;
const newId = () => crypto.randomUUID();

type Model = Extract<SceneObject, { kind: "model" }>;
type MeshObject = Extract<SceneObject, { kind: "mesh" }>;

const num = (label: string, value: number, set: (v: number) => void, step = 0.05) => (
  <label>
    {label}
    <input
      type="number"
      value={Number(value.toFixed(6))}
      step={step}
      onChange={(e) => Number.isFinite(e.target.valueAsNumber) && set(e.target.valueAsNumber)}
    />
  </label>
);

function MaterialPick({
  value,
  onChange,
}: {
  value: MaterialDef;
  onChange: (m: MaterialDef) => void;
}) {
  return (
    <span className="dg-material">
      <select value={value.preset} onChange={(e) => onChange(preset(e.target.value))}>
        {Object.keys(PRESETS).map((p) => (
          <option key={p} value={p}>
            {p.replace("_", " ")}
          </option>
        ))}
      </select>
      <input
        type="color"
        value={value.color}
        onChange={(e) => onChange({ ...value, color: e.target.value })}
        aria-label="Colour"
      />
    </span>
  );
}

function defaultAsset(type: Asset["type"], markers: number): Asset {
  switch (type) {
    case "vehicle":
      return { type, cls: "car", ...VEHICLES.car };
    case "person":
      return { type, height: 1.75, pose: POSES.standing };
    case "furniture":
      return { type, item: "table", ...FURNITURE.table };
    case "weapon":
      return { type, item: "handgun", ...WEAPONS.handgun };
    case "marker":
      return { type, number: markers + 1, size: 0.15 };
  }
}

function assetName(a: Asset): string {
  switch (a.type) {
    case "vehicle":
      return a.cls.replace("_", " ");
    case "person":
      return `Person ${a.height.toFixed(2)} m`;
    case "furniture":
      return a.item;
    case "weapon":
      return a.item.replace("_", " ");
    case "marker":
      return `Marker ${a.number}`;
  }
}

/** Heading of a matrix's x axis about z, degrees. */
const headingOf = (m: number[]) => (Math.atan2(m[1], m[0]) * 180) / Math.PI;

export function SceneBuilder({
  engine,
  diagramList,
  origin,
  requestPick,
  onNotice,
  evidence,
  root,
}: {
  engine: () => Engine | null;
  evidence: EvidenceRecord[];
  /** The project's folder (imported meshes are loaded per project). */
  root: string;
  /** The project's diagrams at their newest revisions. */
  diagramList: DiagramRevision[];
  /** The view's render origin; geometry is rebuilt relative to it when it changes. */
  origin: string;
  /** Ask the view for the next click on the point cloud. */
  requestPick: (hint: string, then: (hit: PickHit) => void) => void;
  onNotice: (m: string | null) => void;
}) {
  const readOnly = useReadOnly();
  const [rev, setRev] = useState<SceneRevision | null | undefined>(undefined);
  const [doc, setDoc] = useState<SceneDoc>(EMPTY_SCENE);
  /** The document as last saved; it differs from `doc` while edits wait to be saved. */
  const [saved, setSaved] = useState<SceneDoc>(EMPTY_SCENE);
  const dirty = doc !== saved;
  const [diagrams, setDiagrams] = useState<Diagrams>(new Map());
  const [meshes, setMeshes] = useState<Meshes>(new Map());
  const [selected, setSelected] = useState<string | null>(null);
  const [move, setMove] = useState<"off" | "translate" | "rotate">("off");
  const [snapOpts, setSnapOpts] = useState({ radius: 0.1, align: false });

  // Load (or not yet create) the project's scene, and the diagrams it can extrude.
  useEffect(() => {
    api
      .scenes()
      .then((list) => {
        const r = list[0] ?? null;
        setRev(r);
        if (r) {
          const d = { ...EMPTY_SCENE, ...r.document };
          setDoc(d);
          setSaved(d);
        }
      })
      .catch((e) => onNotice(String(e)));
  }, [onNotice]);

  // The diagram revisions the scene's objects name.
  const wanted = useMemo(
    () =>
      [
        ...new Set(
          doc.objects.flatMap((o) =>
            o.kind === "extrusion" || o.kind === "roof" ? [o.diagram.revision] : [],
          ),
        ),
      ].filter((r) => !diagrams.has(r)),
    [doc.objects, diagrams],
  );
  useEffect(() => {
    if (!wanted.length) return;
    Promise.all(wanted.map((r) => api.diagramRevision(r))).then(
      (revs) =>
        setDiagrams((m) => {
          const n = new Map(m);
          for (const r of revs) n.set(r.revision_id, r.document as Diagram);
          return n;
        }),
      (e) => onNotice(String(e)),
    );
  }, [wanted, onNotice]);

  // The imported meshes the scene's objects name (each hash-checked by the backend).
  const wantedMeshes = useMemo(
    () =>
      [...new Set(doc.objects.flatMap((o) => (o.kind === "mesh" ? [o.evidence.id] : [])))].filter(
        (id) => !meshes.has(id),
      ),
    [doc.objects, meshes],
  );
  useEffect(() => {
    for (const id of wantedMeshes) {
      const rec = evidence.find((e) => e.id === id);
      if (!rec) {
        onNotice(`The scene names evidence #${id}, which is not in this project.`);
        continue;
      }
      loadEvidenceMesh(rec, root).then(
        (m) => setMeshes((old) => new Map(old).set(id, m)),
        (e) => onNotice(String(e)),
      );
    }
  }, [wantedMeshes, evidence, root, onNotice]);

  // Document → view.
  useEffect(() => {
    const e = engine();
    if (!e) return;
    e.setBuilt(buildScene(doc, diagrams, e.origin, meshes));
  }, [doc, diagrams, meshes, engine, origin]);
  useEffect(() => () => engine()?.setBuilt(null), [engine]);

  const change = useCallback((next: SceneDoc) => setDoc(next), []);
  const update = useCallback(
    (o: SceneObject) =>
      change({ ...doc, objects: doc.objects.map((x) => (x.id === o.id ? o : x)) }),
    [doc, change],
  );

  // Gizmo moves come back as a new matrix.
  useEffect(() => {
    const e = engine();
    if (!e) return;
    e.onModelMoved = (id, matrix) => {
      const o = doc.objects.find((x) => x.id === id);
      if (o?.kind === "model") update({ ...o, matrix, snap: null });
      if (o?.kind === "mesh") update({ ...o, matrix });
    };
    e.attachModel(move === "off" ? null : selected, move === "off" ? "translate" : move);
    return () => {
      e.onModelMoved = null;
    };
  }, [engine, doc, update, selected, move]);

  // Autosave once edits settle.
  const revId = rev?.document_id;
  const revName = rev?.name;
  useEffect(() => {
    if (!dirty || revId === undefined || revName === undefined) return;
    const save = () =>
      api
        .sceneSave(revId, revName, doc)
        .then((r) => {
          setRev(r);
          setSaved(doc);
        })
        .catch((e) => onNotice(String(e)));
    const t = setTimeout(() => void save(), AUTOSAVE_MS);
    const done = whilePending(save);
    return () => {
      clearTimeout(t);
      done();
    };
  }, [dirty, doc, revId, revName, onNotice]);

  if (rev === undefined) return null;
  // In a case package: the scene as built, and its animation to play; nothing to edit.
  if (readOnly)
    return rev === null ? null : (
      <section className="panel-section scene-builder">
        <PanelHeader>
          3D scene <HelpButton topic="scene3d" />
        </PanelHeader>
        <p className="muted">
          {rev.name}, revision {rev.number} (SHA-256 {rev.sha256.slice(0, 16)}…), as saved in the
          case.
        </p>
        <AnimationPanel
          engine={engine}
          sceneId={rev.document_id}
          saving={false}
          doc={doc}
          setAnimation={() => {}}
          evidence={evidence}
          requestPick={requestPick}
          onNotice={onNotice}
        />
      </section>
    );
  if (rev === null)
    return (
      <section className="panel-section">
        <PanelHeader>
          3D scene <HelpButton topic="scene3d" />
        </PanelHeader>
        <p className="muted">
          Extrude diagrams, place models and lights, and set the sun. Everything shares the point
          cloud's coordinates.
        </p>
        <button
          className="primary"
          onClick={() =>
            api.sceneCreate("3D scene", EMPTY_SCENE).then(
              (r) => {
                setRev(r);
                setDoc(r.document);
                setSaved(r.document);
              },
              (e) => onNotice(String(e)),
            )
          }
        >
          Start a 3D scene
        </button>
      </section>
    );

  const sel = doc.objects.find((o) => o.id === selected) ?? null;
  const add = (o: SceneObject) => {
    change({ ...doc, objects: [...doc.objects, o] });
    setSelected(o.id);
  };
  const ref = (d: DiagramRevision): DiagramRef => ({
    id: d.document_id,
    revision: d.revision_id,
    sha256: d.sha256,
    name: d.name,
  });
  const markers = doc.objects.filter((o) => o.kind === "model" && o.asset.type === "marker").length;
  // Rooms available for roofs: in the diagram revisions the scene extrudes.
  const rooms = doc.objects.flatMap((o) =>
    o.kind === "extrusion"
      ? (diagrams.get(o.diagram.revision)?.entities ?? [])
          .filter((e) => e.kind === "room")
          .map((e) => ({
            extrusion: o.id,
            diagram: o.diagram,
            room: e,
            base: o.params.base,
            height: o.params.wallHeight,
          }))
      : [],
  );

  const snap = (m: Model) =>
    requestPick("Click the surface to put the model on.", async (hit) => {
      const e = engine();
      if (!e) return;
      try {
        const s = await api.surfaceAt(hit, snapOpts.radius, e.cameraProject().eye);
        const heading = headingOf(m.matrix);
        let matrix = placeMatrix(s.point, heading);
        if (snapOpts.align) {
          // Turn the model's up (+z) onto the surface normal, keeping its heading.
          const q = new THREE.Quaternion().setFromUnitVectors(
            new THREE.Vector3(0, 0, 1),
            new THREE.Vector3(...s.normal),
          );
          const r = new THREE.Matrix4().makeRotationFromQuaternion(q);
          const h = new THREE.Matrix4().makeRotationZ((heading * Math.PI) / 180);
          const full = r.multiply(h);
          full.elements[12] = s.point[0];
          full.elements[13] = s.point[1];
          full.elements[14] = s.point[2];
          matrix = full.toArray();
        }
        update({ ...m, matrix, snap: { ...s, radius: snapOpts.radius, aligned: snapOpts.align } });
        onNotice(
          `Snapped to a surface fitted to ${s.points} points (RMS ${(s.rms * 1000).toFixed(1)} mm, worst ${(s.max_abs * 1000).toFixed(1)} mm).` +
            (s.rms > 0.005 ? " The surface there isn't flat; check the placement." : ""),
        );
      } catch (err) {
        onNotice(String(err));
      }
    });

  const sunUpdate = async (patch: Partial<SceneDoc["sun"]>) => {
    const sun = { ...doc.sun, ...patch };
    try {
      const t = Date.parse(sun.time) / 1000;
      const s = await api.sunPosition(sun.lat, sun.lon, t);
      sun.computed = {
        azimuth: s.azimuth,
        apparent_elevation: s.apparent_elevation,
        uncertainty: s.uncertainty,
      };
    } catch (e) {
      onNotice(String(e));
      sun.computed = null;
    }
    change({ ...doc, sun });
  };

  return (
    <section className="panel-section scene-builder">
      <PanelHeader>
        3D scene <HelpButton topic="scene3d" />
      </PanelHeader>
      <div className="dg-objects">
        {doc.objects.map((o) => (
          <div key={o.id} className={`dg-object${o.id === selected ? " active" : ""}`}>
            <input
              type="checkbox"
              checked={o.visible}
              onChange={(e) => update({ ...o, visible: e.target.checked })}
              aria-label="Show"
            />
            <button className="link" onClick={() => setSelected(o.id === selected ? null : o.id)}>
              {o.name}
            </button>
            <button
              className="link"
              title="Remove"
              onClick={() => {
                if (selected === o.id) setSelected(null);
                change({ ...doc, objects: doc.objects.filter((x) => x.id !== o.id) });
              }}
            >
              ×
            </button>
          </div>
        ))}
        {doc.objects.length === 0 && <p className="muted">Nothing yet.</p>}
      </div>

      <label>
        Extrude a diagram
        <select
          value=""
          onChange={(e) => {
            const d = diagramList.find((x) => x.document_id === Number(e.target.value));
            if (d)
              add({
                id: newId(),
                kind: "extrusion",
                name: `${d.name} (rev. ${d.number})`,
                visible: true,
                diagram: ref(d),
                params: { ...DEFAULT_EXTRUDE },
                materials: {},
              });
          }}
        >
          <option value="">Choose…</option>
          {diagramList.map((d) => (
            <option key={d.document_id} value={d.document_id}>
              {d.name} (revision {d.number})
            </option>
          ))}
        </select>
      </label>
      {rooms.length > 0 && (
        <label>
          Add a roof
          <select
            value=""
            onChange={(e) => {
              const r = rooms[Number(e.target.value)];
              if (r && r.room.kind === "room") {
                const rect = rectangle(
                  walls(r.room.outline, r.room.thickness, []).map((w) => w.oa),
                );
                add({
                  id: newId(),
                  kind: "roof",
                  name: `Roof (${r.diagram.name})`,
                  visible: true,
                  diagram: r.diagram,
                  extrusion: r.extrusion,
                  room: r.room.id,
                  params: {
                    ...DEFAULT_ROOF,
                    type: rect ? "gable" : "flat",
                    eaves: r.height,
                  },
                  material: preset("roof_tile"),
                });
              }
            }}
          >
            <option value="">Choose a room…</option>
            {rooms.map((r, i) => (
              <option key={i} value={i}>
                {r.diagram.name}, room {i + 1}
              </option>
            ))}
          </select>
        </label>
      )}
      <label>
        Add a model
        <select
          value=""
          onChange={(e) => {
            const type = e.target.value as Asset["type"];
            if (!type) return;
            const asset = defaultAsset(type, markers);
            const at = engine()?.cameraProject().target ?? [0, 0, 0];
            add({
              id: newId(),
              kind: "model",
              name: assetName(asset),
              visible: true,
              asset,
              matrix: placeMatrix(at, 0),
              materials: {},
              snap: null,
            });
          }}
        >
          <option value="">Choose…</option>
          <option value="vehicle">Vehicle</option>
          <option value="person">Person</option>
          <option value="furniture">Furniture</option>
          <option value="weapon">Weapon (generic)</option>
          <option value="marker">Evidence marker</option>
        </select>
      </label>
      {evidence.some(isMesh) && (
        <label>
          Add an imported mesh
          <select
            value=""
            onChange={(e) => {
              const rec = evidence.find((r) => r.id === Number(e.target.value));
              if (!rec) return;
              loadEvidenceMesh(rec, root).then(
                (m) => {
                  const pivot = basePoint(m);
                  add({
                    id: newId(),
                    kind: "mesh",
                    name: rec.contents.meshes[0].name,
                    visible: true,
                    evidence: { id: rec.id, sha256: rec.sha256, name: rec.contents.meshes[0].name },
                    pivot,
                    // At the mesh's own coordinates, as the 3D view shows it; move it from there.
                    matrix: placeMatrix(pivot, 0),
                  });
                },
                (err) => onNotice(String(err)),
              );
            }}
          >
            <option value="">Choose…</option>
            {evidence.filter(isMesh).map((r) => (
              <option key={r.id} value={r.id}>
                #{r.id} {r.contents.meshes[0].name} ({r.contents.format})
              </option>
            ))}
          </select>
        </label>
      )}
      <label>
        Add a light
        <select
          value=""
          onChange={(e) => {
            const t = e.target.value;
            const at = engine()?.cameraProject().target ?? [0, 0, 0];
            const up: [number, number, number] = [at[0], at[1], at[2] + 3];
            const light =
              t === "point"
                ? {
                    type: "point" as const,
                    position: up,
                    color: "#ffffff",
                    intensity: 20,
                    range: 0,
                  }
                : t === "spot"
                  ? {
                      type: "spot" as const,
                      position: up,
                      target: at,
                      color: "#ffffff",
                      intensity: 40,
                      angle: 30,
                    }
                  : t === "directional"
                    ? {
                        type: "directional" as const,
                        direction: [0.3, 0.2, -1] as [number, number, number],
                        color: "#ffffff",
                        intensity: 1.5,
                      }
                    : t === "ambient"
                      ? { type: "ambient" as const, color: "#ffffff", intensity: 0.5 }
                      : null;
            if (light)
              add({ id: newId(), kind: "light", name: `${t} light`, visible: true, light });
          }}
        >
          <option value="">Choose…</option>
          <option value="point">Point</option>
          <option value="spot">Spot</option>
          <option value="directional">Directional</option>
          <option value="ambient">Ambient</option>
        </select>
      </label>

      {sel && (
        <div className="dg-built">
          <h3>{sel.name}</h3>
          {sel.kind === "extrusion" && (
            <>
              <p className="muted">
                From {sel.diagram.name}, revision {sel.diagram.revision} (SHA-256{" "}
                {sel.diagram.sha256.slice(0, 12)}…). Plan positions are the diagram's own.
              </p>
              {num(
                "Base elevation (m)",
                sel.params.base,
                (base) => update({ ...sel, params: { ...sel.params, base } }),
                0.01,
              )}
              <button
                onClick={() =>
                  requestPick(
                    "Click the floor or road surface on the point cloud.",
                    async (hit) => {
                      const e = engine();
                      if (!e) return;
                      try {
                        const s = await api.surfaceAt(hit, snapOpts.radius, e.cameraProject().eye);
                        update({ ...sel, params: { ...sel.params, base: s.point[2] } });
                        onNotice(
                          `Base set from the cloud: ${s.point[2].toFixed(3)} m (surface RMS ${(s.rms * 1000).toFixed(1)} mm).`,
                        );
                      } catch (err) {
                        onNotice(String(err));
                      }
                    },
                  )
                }
              >
                Pick the base on the cloud
              </button>
              {num(
                "Wall height (m)",
                sel.params.wallHeight,
                (wallHeight) =>
                  wallHeight > 0 && update({ ...sel, params: { ...sel.params, wallHeight } }),
              )}
              {num(
                "Floor slab (m)",
                sel.params.floor,
                (floor) => floor >= 0 && update({ ...sel, params: { ...sel.params, floor } }),
                0.01,
              )}
              {num(
                "Painted line width (m)",
                sel.params.lineWidth,
                (lineWidth) =>
                  lineWidth > 0 && update({ ...sel, params: { ...sel.params, lineWidth } }),
                0.01,
              )}
              {(["wall", "floor", "road", "shoulder", "marking"] as Part[]).map((p) => (
                <label key={p} className="inline">
                  {p}
                  <MaterialPick
                    value={
                      sel.materials[p] ??
                      preset(
                        (
                          {
                            wall: "plaster",
                            floor: "concrete",
                            road: "asphalt",
                            shoulder: "concrete",
                            marking: "road_paint",
                          } as const
                        )[p],
                      )
                    }
                    onChange={(m) => update({ ...sel, materials: { ...sel.materials, [p]: m } })}
                  />
                </label>
              ))}
            </>
          )}
          {sel.kind === "roof" && (
            <>
              <label>
                Type
                <select
                  value={sel.params.type}
                  onChange={(e) =>
                    update({ ...sel, params: { ...sel.params, type: e.target.value as RoofType } })
                  }
                >
                  <option value="flat">Flat</option>
                  <option value="shed">Shed</option>
                  <option value="gable">Gable</option>
                  <option value="hip">Hip</option>
                </select>
              </label>
              {num(
                "Eaves above the floor (m)",
                sel.params.eaves,
                (eaves) => update({ ...sel, params: { ...sel.params, eaves } }),
                0.01,
              )}
              {sel.params.type !== "flat" &&
                num(
                  "Pitch (°)",
                  sel.params.pitch,
                  (pitch) =>
                    pitch > 0 && pitch < 80 && update({ ...sel, params: { ...sel.params, pitch } }),
                  1,
                )}
              {num(
                "Overhang (m)",
                sel.params.overhang,
                (overhang) =>
                  overhang >= 0 && update({ ...sel, params: { ...sel.params, overhang } }),
              )}
              {sel.params.type === "flat" &&
                num(
                  "Thickness (m)",
                  sel.params.thickness,
                  (thickness) =>
                    thickness > 0 && update({ ...sel, params: { ...sel.params, thickness } }),
                  0.01,
                )}
              <label className="inline">
                Material{" "}
                <MaterialPick
                  value={sel.material}
                  onChange={(material) => update({ ...sel, material })}
                />
              </label>
            </>
          )}
          {sel.kind === "model" && (
            <ModelEditor
              m={sel}
              update={update}
              move={move}
              setMove={setMove}
              snap={() => snap(sel)}
              snapOpts={snapOpts}
              setSnapOpts={setSnapOpts}
            />
          )}
          {sel.kind === "mesh" && (
            <MeshEditor m={sel} update={update} move={move} setMove={setMove} />
          )}
          {sel.kind === "light" && <LightEditor o={sel} update={update} />}
        </div>
      )}

      <h3>Sun</h3>
      <label className="inline">
        <input
          type="checkbox"
          checked={doc.sun.on}
          onChange={(e) => void sunUpdate({ on: e.target.checked })}
        />
        Sun from place and time
      </label>
      {doc.sun.on && (
        <>
          {num("Latitude (°, north +)", doc.sun.lat, (lat) => void sunUpdate({ lat }), 0.0001)}
          {num("Longitude (°, east +)", doc.sun.lon, (lon) => void sunUpdate({ lon }), 0.0001)}
          <label>
            Date and time (UTC)
            <input
              type="datetime-local"
              value={doc.sun.time.slice(0, 16)}
              onChange={(e) => void sunUpdate({ time: `${e.target.value}:00Z` })}
            />
          </label>
          {num(
            "True north (° anticlockwise from project +y)",
            doc.sun.north,
            (north) => void sunUpdate({ north }),
            0.1,
          )}
          {num("Intensity", doc.sun.intensity, (intensity) => void sunUpdate({ intensity }), 0.1)}
          <label className="inline">
            <input
              type="checkbox"
              checked={doc.sun.shadows}
              onChange={(e) => void sunUpdate({ shadows: e.target.checked })}
            />
            Shadows
          </label>
          {doc.sun.computed && (
            <p className="muted">
              Azimuth {doc.sun.computed.azimuth.toFixed(2)}°, elevation{" "}
              {doc.sun.computed.apparent_elevation.toFixed(2)}° (±{" "}
              {doc.sun.computed.uncertainty.toFixed(2)}°, NOAA solar position with standard
              refraction).
              {doc.sun.computed.apparent_elevation < 0 ? " The sun is below the horizon." : ""}
            </p>
          )}
        </>
      )}
      {num(
        "Fill light",
        doc.ambient,
        (ambient) => ambient >= 0 && change({ ...doc, ambient }),
        0.1,
      )}
      <button
        disabled={dirty}
        title={dirty ? "Waiting for the scene to save" : undefined}
        onClick={async () => {
          const e = engine();
          if (!e) return;
          try {
            const path = await savePath(rev.name, "glb", "glTF binary");
            if (!path) return;
            const bytes = await sceneGlb(doc, diagrams, e.origin, meshes);
            const sha = await api.exportBytes(
              path,
              "3D scene glTF",
              {
                scene: rev.document_id,
                revision: rev.revision_id,
                revision_sha256: rev.sha256,
                origin: e.origin,
              },
              bytes,
            );
            onNotice(`3D scene written as glTF (SHA-256 ${sha}; recorded in the audit log).`);
          } catch (err) {
            onNotice(String(err));
          }
        }}
      >
        Export glTF…
      </button>
      <p className="muted dg-hash">
        Revision {rev.number}
        {dirty ? " (saving…)" : ""} · SHA-256 {rev.sha256.slice(0, 16)}…
      </p>
      <AnimationPanel
        engine={engine}
        sceneId={rev.document_id}
        saving={dirty}
        doc={doc}
        setAnimation={(f) => setDoc((d) => ({ ...d, animation: f(d.animation) }))}
        evidence={evidence}
        requestPick={requestPick}
        onNotice={onNotice}
      />
    </section>
  );
}

function MeshEditor({
  m,
  update,
  move,
  setMove,
}: {
  m: MeshObject;
  update: (o: SceneObject) => void;
  move: "off" | "translate" | "rotate";
  setMove: (m: "off" | "translate" | "rotate") => void;
}) {
  const pos: [number, number, number] = [m.matrix[12], m.matrix[13], m.matrix[14]];
  const [heading, scale] = [headingOf(m.matrix), scaleOf(m.matrix)];
  const place = (p: [number, number, number], h: number, s: number) =>
    update({ ...m, matrix: placeMatrix(p, h, s) });
  return (
    <>
      <p className="muted">
        Imported mesh, evidence #{m.evidence.id} (SHA-256 {m.evidence.sha256.slice(0, 12)}…), in
        metres from its recorded unit. x, y, z is where the bottom centre of its bounds goes; as
        first added it sits at its own coordinates. It can&apos;t be picked or measured.
      </p>
      {num("x (m)", pos[0], (x) => place([x, pos[1], pos[2]], heading, scale), 0.01)}
      {num("y (m)", pos[1], (y) => place([pos[0], y, pos[2]], heading, scale), 0.01)}
      {num("z (m)", pos[2], (z) => place([pos[0], pos[1], z], heading, scale), 0.01)}
      {num("Heading (°, anticlockwise from +x)", heading, (h) => place(pos, h, scale), 1)}
      {num("Scale (×)", scale, (s) => s > 0 && place(pos, heading, s), 0.01)}
      <div className="buttons">
        <button
          className={move === "translate" ? "primary" : ""}
          onClick={() => setMove(move === "translate" ? "off" : "translate")}
        >
          Move
        </button>
        <button
          className={move === "rotate" ? "primary" : ""}
          onClick={() => setMove(move === "rotate" ? "off" : "rotate")}
        >
          Rotate
        </button>
      </div>
    </>
  );
}

function ModelEditor({
  m,
  update,
  move,
  setMove,
  snap,
  snapOpts,
  setSnapOpts,
}: {
  m: Model;
  update: (o: SceneObject) => void;
  move: "off" | "translate" | "rotate";
  setMove: (m: "off" | "translate" | "rotate") => void;
  snap: () => void;
  snapOpts: { radius: number; align: boolean };
  setSnapOpts: (o: { radius: number; align: boolean }) => void;
}) {
  const a = m.asset;
  const setAsset = (asset: Asset) => update({ ...m, asset });
  const pos: [number, number, number] = [m.matrix[12], m.matrix[13], m.matrix[14]];
  const heading = headingOf(m.matrix);
  const place = (p: [number, number, number], h: number) =>
    update({ ...m, matrix: placeMatrix(p, h), snap: null });
  const slots: Slot[] =
    a.type === "vehicle"
      ? ["body", "glass", "tyre"]
      : a.type === "person"
        ? ["skin", "cloth"]
        : a.type === "furniture"
          ? ["wood", "cloth"]
          : a.type === "weapon"
            ? ["metal", "wood"]
            : ["marker"];
  return (
    <>
      {a.type === "vehicle" && (
        <>
          <label>
            Class
            <select
              value={a.cls}
              onChange={(e) => {
                const cls = e.target.value as VehicleClass;
                // A new class starts from its preset; the specification fields go back to
                // the class's derived values.
                setAsset({
                  type: "vehicle",
                  cls,
                  ...VEHICLES[cls],
                });
              }}
            >
              {Object.keys(VEHICLES).map((c) => (
                <option key={c} value={c}>
                  {c.replace("_", " ")}
                </option>
              ))}
            </select>
          </label>
          {num("Length (m)", a.length, (length) => length > 0 && setAsset({ ...a, length }))}
          {num("Width (m)", a.width, (width) => width > 0 && setAsset({ ...a, width }))}
          {num("Height (m)", a.height, (height) => height > 0 && setAsset({ ...a, height }))}
          {num(
            "Wheelbase (m)",
            a.wheelbase,
            (wheelbase) => wheelbase > 0 && wheelbase < a.length && setAsset({ ...a, wheelbase }),
          )}
          {a.cls !== "motorcycle" && a.cls !== "bicycle" && (
            <VehicleSpecFields a={a} setAsset={setAsset} />
          )}
        </>
      )}
      {a.type === "person" && (
        <>
          {num(
            "Height (m)",
            a.height,
            (height) => height > 0.3 && setAsset({ ...a, height }),
            0.01,
          )}
          <label>
            Pose
            <select
              value=""
              onChange={(e) =>
                e.target.value &&
                setAsset({ ...a, pose: POSES[e.target.value as keyof typeof POSES] })
              }
            >
              <option value="">Preset…</option>
              {Object.keys(POSES).map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </label>
          {num(
            "Torso lean (°)",
            a.pose.torso,
            (torso) => setAsset({ ...a, pose: { ...a.pose, torso } }),
            5,
          )}
          {(["hip", "knee", "shoulder", "abduct", "elbow"] as const).map((j) =>
            ([0, 1] as const).map((i) => (
              <span key={`${j}${i}`}>
                {num(
                  `${j} ${i ? "right" : "left"} (°)`,
                  a.pose[j][i],
                  (v) => {
                    const pair = [...a.pose[j]] as [number, number];
                    pair[i] = v;
                    setAsset({ ...a, pose: { ...a.pose, [j]: pair } });
                  },
                  5,
                )}
              </span>
            )),
          )}
          {num(
            "Lying (° onto the back)",
            a.pose.lie,
            (lie) => setAsset({ ...a, pose: { ...a.pose, lie } }),
            5,
          )}
        </>
      )}
      {a.type === "furniture" && (
        <>
          <label>
            Item
            <select
              value={a.item}
              onChange={(e) => {
                const item = e.target.value as FurnitureItem;
                setAsset({ ...a, item, ...FURNITURE[item] });
              }}
            >
              {Object.keys(FURNITURE).map((f) => (
                <option key={f} value={f}>
                  {f}
                </option>
              ))}
            </select>
          </label>
          {num("Length (m)", a.length, (length) => length > 0 && setAsset({ ...a, length }))}
          {num("Width (m)", a.width, (width) => width > 0 && setAsset({ ...a, width }))}
          {num("Height (m)", a.height, (height) => height > 0 && setAsset({ ...a, height }))}
        </>
      )}
      {a.type === "weapon" && (
        <>
          <label>
            Kind
            <select
              value={a.item}
              onChange={(e) => {
                const item = e.target.value as WeaponItem;
                setAsset({ ...a, item, ...WEAPONS[item] });
              }}
            >
              {Object.keys(WEAPONS).map((w) => (
                <option key={w} value={w}>
                  {w.replace("_", " ")}
                </option>
              ))}
            </select>
          </label>
          {num("Length (m)", a.length, (length) => length > 0 && setAsset({ ...a, length }), 0.01)}
        </>
      )}
      {a.type === "marker" && (
        <>
          {num("Number", a.number, (n) => n >= 1 && setAsset({ ...a, number: Math.round(n) }), 1)}
          {num("Size (m)", a.size, (size) => size > 0 && setAsset({ ...a, size }), 0.01)}
        </>
      )}
      {num("x (m)", pos[0], (x) => place([x, pos[1], pos[2]], heading), 0.01)}
      {num("y (m)", pos[1], (y) => place([pos[0], y, pos[2]], heading), 0.01)}
      {num("z (m)", pos[2], (z) => place([pos[0], pos[1], z], heading), 0.01)}
      {num("Heading (°, anticlockwise from +x)", heading, (h) => place(pos, h), 1)}
      <div className="buttons">
        <button
          className={move === "translate" ? "primary" : ""}
          onClick={() => setMove(move === "translate" ? "off" : "translate")}
        >
          Move
        </button>
        <button
          className={move === "rotate" ? "primary" : ""}
          onClick={() => setMove(move === "rotate" ? "off" : "rotate")}
        >
          Rotate
        </button>
      </div>
      <div className="buttons">
        <button onClick={snap}>Snap to the cloud…</button>
      </div>
      {num(
        "Snap radius (m)",
        snapOpts.radius,
        (radius) => radius > 0 && radius <= 1 && setSnapOpts({ ...snapOpts, radius }),
        0.01,
      )}
      <label className="inline">
        <input
          type="checkbox"
          checked={snapOpts.align}
          onChange={(e) => setSnapOpts({ ...snapOpts, align: e.target.checked })}
        />
        Tilt to the surface
      </label>
      {m.snap && (
        <p className="muted">
          On a surface fitted to {m.snap.points} points within {m.snap.radius} m: RMS{" "}
          {(m.snap.rms * 1000).toFixed(1)} mm, worst {(m.snap.max_abs * 1000).toFixed(1)} mm
          {m.snap.aligned ? ", tilted to its normal" : ""}.
        </p>
      )}
      {slots.map((s) => (
        <label key={s} className="inline">
          {s}
          <MaterialPick
            value={
              m.materials[s] ??
              preset(
                (
                  {
                    body: "painted_metal",
                    glass: "glass",
                    tyre: "rubber",
                    skin: "skin",
                    cloth: "fabric",
                    wood: "wood",
                    metal: "metal",
                    marker: "marker",
                  } as const
                )[s],
              )
            }
            onChange={(mat) => update({ ...m, materials: { ...m.materials, [s]: mat } })}
          />
        </label>
      ))}
    </>
  );
}

function LightEditor({
  o,
  update,
}: {
  o: Extract<SceneObject, { kind: "light" }>;
  update: (o: SceneObject) => void;
}) {
  const l = o.light;
  const set = (patch: Record<string, unknown>) =>
    update({ ...o, light: { ...l, ...patch } as typeof l });
  return (
    <>
      <label className="inline">
        Colour{" "}
        <input type="color" value={l.color} onChange={(e) => set({ color: e.target.value })} />
      </label>
      {num("Intensity", l.intensity, (intensity) => intensity >= 0 && set({ intensity }), 0.5)}
      {"position" in l &&
        (["x", "y", "z"] as const).map((k, i) => (
          <span key={k}>
            {num(
              `${k} (m)`,
              l.position[i],
              (v) => {
                const p = [...l.position] as [number, number, number];
                p[i] = v;
                set({ position: p });
              },
              0.1,
            )}
          </span>
        ))}
      {l.type === "spot" &&
        num(
          "Cone half-angle (°)",
          l.angle,
          (angle) => angle > 0 && angle < 90 && set({ angle }),
          1,
        )}
      {l.type === "point" &&
        num("Range (m, 0 = unlimited)", l.range, (range) => range >= 0 && set({ range }), 1)}
    </>
  );
}

/** A four-wheeled vehicle's specification: entered values replace the class's derived ones
 * (shown until then), and the model is rebuilt from them. */
function VehicleSpecFields({
  a,
  setAsset,
}: {
  a: Extract<Asset, { type: "vehicle" }>;
  setAsset: (a: Asset) => void;
}) {
  const v = vehicleSpec(a);
  const problem = vehicleProblem(a);
  const field = (
    label: string,
    key: "frontOverhang" | "trackFront" | "trackRear" | "tyreDiameter" | "mass" | "cgHeight",
    shown: number | null,
    step = 0.01,
  ) => (
    <label>
      {label}
      <input
        type="number"
        step={step}
        placeholder={shown === null ? "not entered" : undefined}
        value={a[key] ?? (shown === null ? "" : Number(shown.toFixed(4)))}
        className={a[key] === undefined ? "derived" : ""}
        onChange={(e) =>
          setAsset({
            ...a,
            [key]: Number.isFinite(e.target.valueAsNumber) ? e.target.valueAsNumber : undefined,
          })
        }
      />
    </label>
  );
  return (
    <>
      {field("Front overhang (m)", "frontOverhang", v.frontOverhang)}
      <p className="muted">
        Rear overhang {v.rearOverhang.toFixed(3)} m (length − wheelbase − front overhang)
      </p>
      {field("Front track (m)", "trackFront", v.trackFront)}
      {field("Rear track (m)", "trackRear", v.trackRear)}
      {field("Tyre diameter (m)", "tyreDiameter", v.tyreDiameter)}
      {field("Mass (kg)", "mass", v.mass, 10)}
      {field("Centre of gravity height (m)", "cgHeight", v.cgHeight)}
      {problem && <p className="error">{problem}</p>}
      <p className="muted">
        Enter the vehicle&apos;s specification for crash reconstruction. Values in grey are derived
        from the class until entered.
      </p>
    </>
  );
}
