import type { BboxGeometry } from "../types";
import type { Point, Size } from "./viewport";

export type BoxResizeHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export type BoxHandle = BoxResizeHandle | "move";

export const BOX_RESIZE_HANDLES: BoxResizeHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

export function isBoxCreationDrag(start: Point, end: Point, minimumScreenDistance = 3): boolean {
  return Math.abs(end.x - start.x) >= minimumScreenDistance
    && Math.abs(end.y - start.y) >= minimumScreenDistance;
}

export function boxFromCorners(start: Point, end: Point): BboxGeometry {
  return {
    x: Math.min(start.x, end.x),
    y: Math.min(start.y, end.y),
    width: Math.abs(end.x - start.x),
    height: Math.abs(end.y - start.y),
  };
}

export function clampPointToImage(point: Point, image: Size): Point {
  return { x: Math.min(image.width, Math.max(0, point.x)), y: Math.min(image.height, Math.max(0, point.y)) };
}

export function moveBox(box: BboxGeometry, delta: Point, image: Size): BboxGeometry {
  return {
    ...box,
    x: Math.min(image.width - box.width, Math.max(0, box.x + delta.x)),
    y: Math.min(image.height - box.height, Math.max(0, box.y + delta.y)),
  };
}

export function resizeBox(box: BboxGeometry, handle: BoxResizeHandle, point: Point, image: Size, minimum = 1): BboxGeometry {
  const right = box.x + box.width;
  const bottom = box.y + box.height;
  const target = clampPointToImage(point, image);
  const left = handle.includes("w") ? Math.min(target.x, right - minimum) : box.x;
  const top = handle.includes("n") ? Math.min(target.y, bottom - minimum) : box.y;
  const nextRight = handle.includes("e") ? Math.max(target.x, box.x + minimum) : right;
  const nextBottom = handle.includes("s") ? Math.max(target.y, box.y + minimum) : bottom;
  return { x: left, y: top, width: nextRight - left, height: nextBottom - top };
}

export function boxHandlePoint(box: BboxGeometry, handle: BoxResizeHandle): Point {
  const centerX = box.x + box.width / 2;
  const centerY = box.y + box.height / 2;
  switch (handle) {
    case "nw": return { x: box.x, y: box.y };
    case "n": return { x: centerX, y: box.y };
    case "ne": return { x: box.x + box.width, y: box.y };
    case "e": return { x: box.x + box.width, y: centerY };
    case "se": return { x: box.x + box.width, y: box.y + box.height };
    case "s": return { x: centerX, y: box.y + box.height };
    case "sw": return { x: box.x, y: box.y + box.height };
    case "w": return { x: box.x, y: centerY };
  }
}

export function boxMoveHandlePoint(box: BboxGeometry, offset: number): Point {
  return { x: box.x + box.width / 2, y: box.y - offset };
}

export function boxHandleAt(point: Point, box: BboxGeometry, tolerance: number, moveOffset: number): BoxHandle | null {
  const resize = BOX_RESIZE_HANDLES.find((handle) => {
    const target = boxHandlePoint(box, handle);
    return Math.hypot(point.x - target.x, point.y - target.y) <= tolerance;
  });
  if (resize) return resize;
  const move = boxMoveHandlePoint(box, moveOffset);
  return Math.hypot(point.x - move.x, point.y - move.y) <= tolerance ? "move" : null;
}
