// Parameters of a selected room or road; every edit rebuilds its geometry (builders.ts).
import { DASH, road, room, type Opening, type RoadParams } from "./builders";
import type { Entity } from "./model";

type Room = Extract<Entity, { kind: "room" }>;
type Road = Extract<Entity, { kind: "road" }>;

export const DEFAULT_ROAD: RoadParams = {
  lanes: [1, 1],
  laneWidth: 3.5,
  shoulder: 1,
  centre: "dashed",
};
export const DEFAULT_WALL = 0.2;

const num = (label: string, value: number, set: (v: number) => void, step = 0.05, min = 0) => (
  <label>
    {label}
    <input
      type="number"
      value={value}
      step={step}
      min={min}
      onChange={(e) => Number.isFinite(e.target.valueAsNumber) && set(e.target.valueAsNumber)}
    />
  </label>
);

export function BuiltPanel({
  entity,
  onChange,
}: {
  entity: Room | Road;
  onChange: (e: Room | Road) => void;
}) {
  if (entity.kind === "room") {
    const set = (thickness: number, openings: Opening[]) =>
      onChange({
        ...entity,
        thickness,
        openings,
        geometry: room(entity.outline, thickness, openings),
      });
    const setOpening = (i: number, o: Partial<Opening>) =>
      set(
        entity.thickness,
        entity.openings.map((x, j) => (j === i ? { ...x, ...o } : x)),
      );
    const walls = entity.outline.length;
    return (
      <div className="dg-built">
        <h3>Room</h3>
        {num("Wall thickness (m)", entity.thickness, (t) => t > 0 && set(t, entity.openings))}
        {entity.openings.map((o, i) => (
          <fieldset key={i}>
            <legend>
              {o.kind === "door" ? "Door" : "Window"} {i + 1}
            </legend>
            <label>
              Kind
              <select
                value={o.kind}
                onChange={(e) => setOpening(i, { kind: e.target.value as Opening["kind"] })}
              >
                <option value="door">Door</option>
                <option value="window">Window</option>
              </select>
            </label>
            <label>
              Wall (1–{walls})
              <input
                type="number"
                min={1}
                max={walls}
                value={o.wall + 1}
                onChange={(e) =>
                  setOpening(i, {
                    wall: Math.min(walls, Math.max(1, e.target.valueAsNumber || 1)) - 1,
                  })
                }
              />
            </label>
            {num("From wall start (m)", o.at, (at) => setOpening(i, { at }))}
            {num("Width (m)", o.width, (width) => width > 0 && setOpening(i, { width }))}
            {o.kind === "door" && (
              <label>
                Hinge
                <select
                  value={o.hinge}
                  onChange={(e) => setOpening(i, { hinge: e.target.value as Opening["hinge"] })}
                >
                  <option value="near">Near side</option>
                  <option value="far">Far side</option>
                </select>
              </label>
            )}
            <button
              onClick={() =>
                set(
                  entity.thickness,
                  entity.openings.filter((_, j) => j !== i),
                )
              }
            >
              Remove
            </button>
          </fieldset>
        ))}
        <button
          onClick={() =>
            set(entity.thickness, [
              ...entity.openings,
              { kind: "door", wall: 0, at: 0.2, width: 0.9, hinge: "near" },
            ])
          }
        >
          Add door or window
        </button>
        <p className="muted">
          Walls are numbered from the first corner clicked. An opening that doesn't fit its wall is
          not drawn.
        </p>
      </div>
    );
  }
  const set = (r: Partial<RoadParams>) => {
    const next = { ...entity.road, ...r };
    onChange({ ...entity, road: next, geometry: road(entity.centreline, next) });
  };
  return (
    <div className="dg-built">
      <h3>Road</h3>
      {num(
        "Lanes, left of the drawn direction",
        entity.road.lanes[0],
        (n) => set({ lanes: [Math.max(0, Math.round(n)), entity.road.lanes[1]] }),
        1,
      )}
      {num(
        "Lanes, right",
        entity.road.lanes[1],
        (n) => set({ lanes: [entity.road.lanes[0], Math.max(0, Math.round(n))] }),
        1,
      )}
      {num(
        "Lane width (m)",
        entity.road.laneWidth,
        (laneWidth) => laneWidth > 0 && set({ laneWidth }),
      )}
      {num("Shoulder (m)", entity.road.shoulder, (shoulder) => set({ shoulder }))}
      {num("Curve radius at corners (m)", entity.road.radius ?? 0, (radius) => set({ radius }), 1)}
      {num(
        "Broken line: painted (m)",
        (entity.road.dash ?? DASH)[0],
        (on) => on > 0 && set({ dash: [on, (entity.road.dash ?? DASH)[1]] }),
        0.5,
      )}
      {num(
        "Broken line: gap (m)",
        (entity.road.dash ?? DASH)[1],
        (off) => off >= 0 && set({ dash: [(entity.road.dash ?? DASH)[0], off] }),
        0.5,
      )}
      <label>
        Centre line
        <select
          value={entity.road.centre}
          onChange={(e) => set({ centre: e.target.value as RoadParams["centre"] })}
        >
          <option value="dashed">Broken</option>
          <option value="solid">Solid</option>
          <option value="double">Double solid</option>
          <option value="none">None</option>
        </select>
      </label>
      <p className="muted">
        Broken lines default to 3 m painted, 9 m gap; set the local standard or the measured
        pattern. A corner too short for the radius gets the largest curve that fits.
      </p>
    </div>
  );
}
