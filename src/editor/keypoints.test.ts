import { describe, expect, it } from "vitest";
import type { KeypointGeometry } from "../types";
import {
  COCO_PERSON, keypointIndexAt, moveKeypointObject, nextUnlabeled, parseKeypointNames, setKeypoint, UNLABELED,
} from "./keypoints";

const geometry: KeypointGeometry = {
  bbox: { x: 10, y: 10, width: 40, height: 40 },
  keypoints: [{ x: 20, y: 20, v: 2 }, UNLABELED, { x: 45, y: 5, v: 1 }],
};

describe("keypoint geometry", () => {
  it("parses comma- or line-separated names", () => {
    expect(parseKeypointNames(" head, tail\nfin ,, ")).toEqual(["head", "tail", "fin"]);
  });

  it("ships the COCO person schema with valid zero-based pairs", () => {
    expect(COCO_PERSON.names).toHaveLength(17);
    expect(COCO_PERSON.skeleton.flat().every((index) => index >= 0 && index < 17)).toBe(true);
  });

  it("pads to the schema count and normalizes unlabeled points", () => {
    const placed = setKeypoint({ ...geometry, keypoints: [] }, 3, 1, { x: 5, y: 6, v: 2 });
    expect(placed.keypoints).toEqual([UNLABELED, { x: 5, y: 6, v: 2 }, UNLABELED]);
    expect(setKeypoint(placed, 3, 1, { x: 5, y: 6, v: 0 }).keypoints[1]).toEqual(UNLABELED);
  });

  it("advances only to unlabeled slots and stops at the end", () => {
    expect(nextUnlabeled(geometry, 3, 0)).toBe(1);
    expect(nextUnlabeled(geometry, 3, 1)).toBeNull();
    expect(nextUnlabeled(geometry, 5, 2)).toBe(3);
  });

  it("hits the nearest labeled point and ignores unlabeled slots", () => {
    expect(keypointIndexAt({ x: 21, y: 20 }, geometry, 4)).toBe(0);
    expect(keypointIndexAt({ x: 0, y: 0 }, geometry, 4)).toBeNull();
  });

  it("moves the box with its points and keeps points on the image", () => {
    const moved = moveKeypointObject(geometry, { x: -100, y: -2 }, { width: 100, height: 100 });
    expect(moved.bbox).toEqual({ x: 0, y: 8, width: 40, height: 40 });
    expect(moved.keypoints[2]).toEqual({ x: 35, y: 3, v: 1 });
    expect(moved.keypoints[1]).toEqual(UNLABELED);
    expect(moveKeypointObject(geometry, { x: 0, y: -10 }, { width: 100, height: 100 }).keypoints[2].y).toBe(0);
  });
});
