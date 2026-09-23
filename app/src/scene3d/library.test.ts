import { describe, expect, it } from "vitest";
import {
  FURNITURE,
  POSES,
  STATURE,
  vehicleProblem,
  vehicleSpec,
  VEHICLES,
  WEAPONS,
  bounds,
  build,
  type FurnitureItem,
  type VehicleClass,
  type WeaponItem,
} from "./library";

const size = (m: ReturnType<typeof build>) => {
  const [lo, hi] = bounds(m);
  return { lo, hi, x: hi[0] - lo[0], y: hi[1] - lo[1], z: hi[2] - lo[2] };
};

describe("asset library", () => {
  it.each(Object.keys(VEHICLES) as VehicleClass[])(
    "a %s is as long, wide and tall as asked",
    (cls) => {
      // Non-default dimensions, to show they are used.
      const d = { ...VEHICLES[cls] };
      d.length *= 1.1;
      d.height *= 0.95;
      const s = size(build({ type: "vehicle", cls, ...d }));
      expect(s.x).toBeCloseTo(d.length, 2);
      expect(s.z).toBeCloseTo(d.height, 1);
      expect(Math.abs(s.y - d.width)).toBeLessThan(0.02);
      expect(s.lo[2]).toBeCloseTo(0, 6); // on the ground
      // Centred front to back.
      expect(s.lo[0] + s.hi[0]).toBeCloseTo(0, 2);
    },
  );

  it("places a vehicle's wheels by its specification", () => {
    const a = {
      type: "vehicle" as const,
      cls: "car" as const,
      length: 4.9,
      width: 1.85,
      height: 1.45,
      wheelbase: 2.85,
      frontOverhang: 0.95,
      trackFront: 1.58,
      trackRear: 1.56,
      tyreDiameter: 0.66,
    };
    const tyre = build(a).tyre!;
    const xs = tyre.positions.filter((_, i) => i % 3 === 0);
    const ys = tyre.positions.filter((_, i) => i % 3 === 1);
    const zs = tyre.positions.filter((_, i) => i % 3 === 2);
    const front = 4.9 / 2 - 0.95;
    // Along the car: the front axle plus a tyre radius, the rear axle minus one.
    expect(Math.max(...xs)).toBeCloseTo(front + 0.33, 6);
    expect(Math.min(...xs)).toBeCloseTo(front - 2.85 - 0.33, 6);
    // Across: the wider (front) track plus half a tyre's width.
    expect(Math.max(...ys)).toBeCloseTo(1.58 / 2 + 0.13, 6);
    expect(Math.max(...zs)).toBeCloseTo(0.66, 6);
    expect(vehicleSpec(a).rearOverhang).toBeCloseTo(4.9 - 2.85 - 0.95, 12);
    expect(vehicleProblem(a)).toBeNull();
    expect(vehicleProblem({ ...a, frontOverhang: 2.2 })).toMatch(/length/);
    expect(vehicleProblem({ ...a, trackFront: 1.8 })).toMatch(/width/);
  });

  it("derives a missing specification from the class, as older scenes were built", () => {
    const a = { type: "vehicle" as const, cls: "suv" as const, ...VEHICLES.suv };
    const v = vehicleSpec(a);
    expect(v.frontOverhang).toBeCloseTo((4.8 - 2.85) / 2, 12);
    expect(v.rearOverhang).toBeCloseTo(v.frontOverhang, 12);
    expect(v.trackFront).toBeCloseTo(1.95 - 0.3, 12);
  });

  it("stands a person exactly as tall as asked, and lays them down", () => {
    const standing = size(build({ type: "person", height: 1.78, pose: POSES.standing }));
    expect(standing.z).toBeCloseTo(1.78, 9);
    expect(standing.lo[2]).toBeCloseTo(0, 9);
    const lying = size(build({ type: "person", height: 1.78, pose: POSES.lying }));
    expect(lying.x).toBeCloseTo(1.78, 9);
    expect(lying.z).toBeLessThan(0.4);
    expect(lying.lo[2]).toBeCloseTo(0, 9);
    const sitting = size(build({ type: "person", height: 1.78, pose: POSES.sitting }));
    expect(sitting.z).toBeLessThan(1.4);
  });

  it("follows Drillis and Contini's proportions", () => {
    // Shoulder width across the arms, and the chin below the head's top.
    const standing = size(
      build({ type: "person", height: 1.0, pose: { ...POSES.standing, abduct: [0, 0] } }),
    );
    expect(standing.y).toBeCloseTo(STATURE.shoulderWidth, 2);
    expect(standing.x).toBeGreaterThan(STATURE.footLength * 0.95);
  });

  it.each(Object.keys(FURNITURE) as FurnitureItem[])("a %s fills its box", (item) => {
    const d = FURNITURE[item];
    const s = size(build({ type: "furniture", item, ...d }));
    expect(s.x).toBeCloseTo(d.length, 9);
    expect(s.y).toBeCloseTo(d.width, 9);
    expect(s.z).toBeCloseTo(d.height, 9);
  });

  it.each(Object.keys(WEAPONS) as WeaponItem[])(
    "a %s is as long as asked and lies flat",
    (item) => {
      const s = size(build({ type: "weapon", item, length: WEAPONS[item].length }));
      expect(s.x).toBeCloseTo(WEAPONS[item].length, 9);
      expect(s.lo[2]).toBeCloseTo(0, 12);
    },
  );

  it("makes an evidence tent of the given size", () => {
    const s = size(build({ type: "marker", number: 7, size: 0.12 }));
    expect(s.z).toBeCloseTo(0.12, 12);
  });
});
