import { describe, expect, it } from "vitest";
import { hitTestTopmost, pointInPolygon } from "./hitTest";

describe("geometry hit testing", () => {
  it("detects points inside a polygon", () => {
    const polygon = [{ x: 0, y: 0 }, { x: 80, y: 0 }, { x: 40, y: 60 }];
    expect(pointInPolygon({ x: 40, y: 20 }, polygon)).toBe(true);
    expect(pointInPolygon({ x: 70, y: 55 }, polygon)).toBe(false);
  });

  it("returns the topmost overlapping geometry", () => {
    const result = hitTestTopmost({ x: 25, y: 25 }, [
      { id: "back", kind: "bbox", rect: { x: 0, y: 0, width: 50, height: 50 } },
      { id: "front", kind: "bbox", rect: { x: 20, y: 20, width: 50, height: 50 } },
    ], 0);
    expect(result).toBe("front");
  });
});
