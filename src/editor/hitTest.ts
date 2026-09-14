import type { Point } from "./viewport";

export interface RectGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface SelectableGeometry {
  id: string;
  kind: "bbox" | "polygon";
  rect?: RectGeometry;
  points?: Point[];
}

export function pointInPolygon(point: Point, polygon: Point[]): boolean {
  let inside = false;
  for (let index = 0, previous = polygon.length - 1; index < polygon.length; previous = index++) {
    const currentPoint = polygon[index];
    const previousPoint = polygon[previous];
    const crosses = (currentPoint.y > point.y) !== (previousPoint.y > point.y)
      && point.x < ((previousPoint.x - currentPoint.x) * (point.y - currentPoint.y))
        / (previousPoint.y - currentPoint.y) + currentPoint.x;
    if (crosses) inside = !inside;
  }
  return inside;
}

export function distanceToSegment(point: Point, start: Point, end: Point): number {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return Math.hypot(point.x - start.x, point.y - start.y);
  const projection = Math.max(0, Math.min(1,
    ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared,
  ));
  return Math.hypot(point.x - (start.x + projection * dx), point.y - (start.y + projection * dy));
}

export function hitTestGeometry(point: Point, geometry: SelectableGeometry, tolerance: number): boolean {
  if (geometry.kind === "bbox" && geometry.rect) {
    const { x, y, width, height } = geometry.rect;
    return point.x >= x - tolerance && point.x <= x + width + tolerance
      && point.y >= y - tolerance && point.y <= y + height + tolerance;
  }
  if (geometry.kind === "polygon" && geometry.points && geometry.points.length >= 3) {
    if (pointInPolygon(point, geometry.points)) return true;
    return geometry.points.some((start, index) =>
      distanceToSegment(point, start, geometry.points![(index + 1) % geometry.points!.length]) <= tolerance,
    );
  }
  return false;
}

export function hitTestTopmost(
  point: Point,
  geometries: SelectableGeometry[],
  tolerance: number,
): string | null {
  for (let index = geometries.length - 1; index >= 0; index -= 1) {
    if (hitTestGeometry(point, geometries[index], tolerance)) return geometries[index].id;
  }
  return null;
}
