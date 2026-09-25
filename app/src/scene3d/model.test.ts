import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { UNITS } from "../format";
import { fileToProject, meshMatrix, placeMatrix, scaleOf } from "./model";

const apply = (m: number[], p: [number, number, number]) =>
  new THREE.Vector3(...p).applyMatrix4(new THREE.Matrix4().fromArray(m)).toArray();
const close = (a: number[], b: number[]) => a.forEach((v, i) => expect(v).toBeCloseTo(b[i], 12));
const metres = (u: string) => UNITS.find((x) => x.value === u)!.metres;

describe("mesh file to project frame", () => {
  it("scales by the recorded unit, z up files unturned", () => {
    close(apply(fileToProject(metres("millimeter"), false), [1000, 2000, 500]), [1, 2, 0.5]);
    close(apply(fileToProject(metres("foot"), false), [10, 0, 0]), [3.048, 0, 0]);
    close(apply(fileToProject(metres("us_survey_foot"), false), [3937, 0, 0]), [1200, 0, 0]);
  });

  it("turns glTF's y up onto the project's z up", () => {
    const m = fileToProject(1, true);
    close(apply(m, [0, 1, 0]), [0, 0, 1]); // glTF up → project up
    close(apply(m, [0, 0, 1]), [0, -1, 0]); // glTF +z → project −y
    close(apply(m, [1, 0, 0]), [1, 0, 0]);
    // The scene export turns −90° about x: the round trip is the identity.
    const back = new THREE.Matrix4().makeRotationX(-Math.PI / 2);
    close(
      new THREE.Vector3(2, 3, 4)
        .applyMatrix4(new THREE.Matrix4().fromArray(m))
        .applyMatrix4(back)
        .toArray(),
      [2, 3, 4],
    );
  });
});

describe("placement", () => {
  it("scales, turns about z, then moves", () => {
    const m = placeMatrix([10, 20, 1], 90, 2);
    close(apply(m, [1, 0, 0]), [10, 22, 1]);
    close(apply(m, [0, 0, 1]), [10, 20, 3]);
    expect(scaleOf(m)).toBeCloseTo(2, 12);
    expect(placeMatrix([1, 2, 3], 30)).toEqual(placeMatrix([1, 2, 3], 30, 1));
  });
});

describe("exemplar meshes", () => {
  it("a measured mesh keeps its size; an exemplar is fitted along its own axes", () => {
    expect(meshMatrix([1, 2, 3], 30, [4, 2, 1.5], null)).toEqual(placeMatrix([1, 2, 3], 30));
    const m = meshMatrix([0, 0, 0], 90, [4, 2, 1.5], { size: [4.8, 1.9, 1.5] });
    // Its x axis (length 4 → 4.8) now runs along project +y after the 90° turn.
    expect(Math.hypot(m[0], m[1], m[2])).toBeCloseTo(1.2, 12);
    expect(m[1]).toBeCloseTo(1.2, 12);
    expect(Math.hypot(m[4], m[5], m[6])).toBeCloseTo(0.95, 12);
    expect(m[10]).toBeCloseTo(1, 12);
  });
});
