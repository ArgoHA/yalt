import { describe, expect, it } from "vitest";
import { geometryWarnings, movePolygon, movePolygonVertex, polygonArea } from "./polygons";

describe("polygon geometry", () => {
  it("computes area independent of winding", () => {
    expect(polygonArea([{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 0, y: 10 }])).toBe(50);
    expect(polygonArea([{ x: 0, y: 10 }, { x: 10, y: 0 }, { x: 0, y: 0 }])).toBe(50);
  });

  it("keeps moved contours inside the image", () => {
    const moved = movePolygon({ polygons: [[{ x: 5, y: 5 }, { x: 10, y: 5 }, { x: 5, y: 10 }]] }, { x: -20, y: 50 }, { width: 30, height: 30 });
    expect(moved.polygons[0]).toEqual([{ x: 0, y: 25 }, { x: 5, y: 25 }, { x: 0, y: 30 }]);
  });

  it("moves one vertex and warns about crossed edges", () => {
    const geometry = { polygons: [[{ x: 0, y: 0 }, { x: 10, y: 10 }, { x: 0, y: 10 }, { x: 10, y: 0 }]] };
    expect(geometryWarnings(geometry)).toContain("Self-intersecting contour");
    expect(movePolygonVertex(geometry, { polygonIndex: 0, pointIndex: 0 }, { x: -4, y: 20 }, { width: 12, height: 12 }).polygons[0][0]).toEqual({ x: 0, y: 12 });
  });
});
