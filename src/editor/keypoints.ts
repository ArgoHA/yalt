import type { Keypoint, KeypointGeometry, KeypointSchema } from "../types";
import type { Point, Size } from "./viewport";

export const UNLABELED: Keypoint = { x: 0, y: 0, v: 0 };

/** COCO person keypoints in COCO order, skeleton converted to zero-based pairs. */
export const COCO_PERSON: KeypointSchema = {
  names: [
    "nose", "left_eye", "right_eye", "left_ear", "right_ear", "left_shoulder", "right_shoulder",
    "left_elbow", "right_elbow", "left_wrist", "right_wrist", "left_hip", "right_hip",
    "left_knee", "right_knee", "left_ankle", "right_ankle",
  ],
  skeleton: [
    [15, 13], [13, 11], [16, 14], [14, 12], [11, 12], [5, 11], [6, 12], [5, 6], [5, 7], [6, 8],
    [7, 9], [8, 10], [1, 2], [0, 1], [0, 2], [1, 3], [2, 4], [3, 5], [4, 6],
  ],
};

/** Names are separated by commas or line breaks. */
export function parseKeypointNames(text: string): string[] {
  return text.split(/[,\n]/).map((name) => name.trim()).filter(Boolean);
}

export function withKeypointCount(geometry: KeypointGeometry, count: number): KeypointGeometry {
  if (geometry.keypoints.length === count) return geometry;
  return { ...geometry, keypoints: Array.from({ length: count }, (_, index) => geometry.keypoints[index] ?? UNLABELED) };
}

export function setKeypoint(geometry: KeypointGeometry, count: number, index: number, keypoint: Keypoint): KeypointGeometry {
  const keypoints = withKeypointCount(geometry, count).keypoints.slice();
  keypoints[index] = keypoint.v > 0 ? keypoint : UNLABELED;
  return { ...geometry, keypoints };
}

export function isLabeled(geometry: KeypointGeometry, index: number): boolean {
  return (geometry.keypoints[index]?.v ?? 0) > 0;
}

export function labeledCount(geometry: KeypointGeometry): number {
  return geometry.keypoints.filter((point) => point.v > 0).length;
}

/** The first unlabeled slot after `index`, or null when nothing is left to place. */
export function nextUnlabeled(geometry: KeypointGeometry, count: number, index: number): number | null {
  for (let next = index + 1; next < count; next += 1) if (!isLabeled(geometry, next)) return next;
  return null;
}

/** The nearest labeled point within `tolerance`, so overlapping joints pick the closest. */
export function keypointIndexAt(point: Point, geometry: KeypointGeometry, tolerance: number): number | null {
  let found: number | null = null;
  let nearest = tolerance;
  geometry.keypoints.forEach((keypoint, index) => {
    if (keypoint.v === 0) return;
    const distance = Math.hypot(point.x - keypoint.x, point.y - keypoint.y);
    if (distance <= nearest) { found = index; nearest = distance; }
  });
  return found;
}

/** Moves the box and its labeled points together, keeping all of them on the image. */
export function moveKeypointObject(geometry: KeypointGeometry, delta: Point, image: Size): KeypointGeometry {
  const { bbox } = geometry;
  let left = bbox.x;
  let top = bbox.y;
  let right = bbox.x + bbox.width;
  let bottom = bbox.y + bbox.height;
  for (const point of geometry.keypoints) {
    if (point.v === 0) continue;
    left = Math.min(left, point.x); top = Math.min(top, point.y);
    right = Math.max(right, point.x); bottom = Math.max(bottom, point.y);
  }
  const dx = Math.min(image.width - right, Math.max(-left, delta.x));
  const dy = Math.min(image.height - bottom, Math.max(-top, delta.y));
  return {
    bbox: { ...bbox, x: bbox.x + dx, y: bbox.y + dy },
    keypoints: geometry.keypoints.map((point) => point.v > 0 ? { ...point, x: point.x + dx, y: point.y + dy } : point),
  };
}
