import type { PolygonGeometry, PolygonPoint } from "../types";
import type { Size } from "./viewport";

export interface PolygonVertex {
  polygonIndex: number;
  pointIndex: number;
}

export function polygonArea(points: PolygonPoint[]): number {
  if (points.length < 3) return 0;
  return Math.abs(points.reduce((sum, point, index) => {
    const next = points[(index + 1) % points.length];
    return sum + point.x * next.y - next.x * point.y;
  }, 0)) / 2;
}

export function geometryArea(geometry: PolygonGeometry): number {
  return geometry.polygons.reduce((sum, polygon) => sum + polygonArea(polygon), 0);
}

export function polygonVertexAt(point: PolygonPoint, geometry: PolygonGeometry, tolerance: number): PolygonVertex | null {
  for (let polygonIndex = geometry.polygons.length - 1; polygonIndex >= 0; polygonIndex -= 1) {
    const polygon = geometry.polygons[polygonIndex];
    for (let pointIndex = polygon.length - 1; pointIndex >= 0; pointIndex -= 1) {
      const candidate = polygon[pointIndex];
      if (Math.hypot(candidate.x - point.x, candidate.y - point.y) <= tolerance) {
        return { polygonIndex, pointIndex };
      }
    }
  }
  return null;
}

export function movePolygon(geometry: PolygonGeometry, delta: PolygonPoint, bounds: Size): PolygonGeometry {
  const points = geometry.polygons.flat();
  const minX = Math.min(...points.map((point) => point.x));
  const maxX = Math.max(...points.map((point) => point.x));
  const minY = Math.min(...points.map((point) => point.y));
  const maxY = Math.max(...points.map((point) => point.y));
  const dx = Math.max(-minX, Math.min(bounds.width - maxX, delta.x));
  const dy = Math.max(-minY, Math.min(bounds.height - maxY, delta.y));
  return { polygons: geometry.polygons.map((polygon) => polygon.map((point) => ({ x: point.x + dx, y: point.y + dy }))) };
}

export function movePolygonVertex(
  geometry: PolygonGeometry,
  vertex: PolygonVertex,
  point: PolygonPoint,
  bounds: Size,
): PolygonGeometry {
  return {
    polygons: geometry.polygons.map((polygon, polygonIndex) => polygon.map((current, pointIndex) =>
      polygonIndex === vertex.polygonIndex && pointIndex === vertex.pointIndex
        ? { x: Math.max(0, Math.min(bounds.width, point.x)), y: Math.max(0, Math.min(bounds.height, point.y)) }
        : current,
    )),
  };
}

export function geometryWarnings(geometry: PolygonGeometry): string[] {
  const warnings: string[] = [];
  if (geometry.polygons.some((polygon) => polygonArea(polygon) < 4)) warnings.push("Very small area");
  if (geometry.polygons.some(polygonSelfIntersects)) warnings.push("Self-intersecting contour");
  return warnings;
}

export function polygonSelfIntersects(points: PolygonPoint[]): boolean {
  if (points.length < 4) return false;
  for (let first = 0; first < points.length; first += 1) {
    const firstNext = (first + 1) % points.length;
    for (let second = first + 1; second < points.length; second += 1) {
      const secondNext = (second + 1) % points.length;
      if (first === second || firstNext === second || secondNext === first) continue;
      if (segmentsIntersect(points[first], points[firstNext], points[second], points[secondNext])) return true;
    }
  }
  return false;
}

function segmentsIntersect(a: PolygonPoint, b: PolygonPoint, c: PolygonPoint, d: PolygonPoint): boolean {
  const cross = (p: PolygonPoint, q: PolygonPoint, r: PolygonPoint) =>
    (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
  const abC = cross(a, b, c);
  const abD = cross(a, b, d);
  const cdA = cross(c, d, a);
  const cdB = cross(c, d, b);
  return ((abC > 0 && abD < 0) || (abC < 0 && abD > 0))
    && ((cdA > 0 && cdB < 0) || (cdA < 0 && cdB > 0));
}
