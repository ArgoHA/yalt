import { describe, expect, it } from "vitest";
import { boxFromCorners, boxHandleAt, boxHandlePoint, isBoxCreationDrag, moveBox, resizeBox } from "./boxes";

describe("box geometry", () => {
  it("normalizes reverse click order", () => {
    expect(boxFromCorners({ x: 80, y: 60 }, { x: 20, y: 10 })).toEqual({ x: 20, y: 10, width: 60, height: 50 });
  });
  it("keeps moved boxes inside the image", () => {
    expect(moveBox({ x: 10, y: 10, width: 30, height: 20 }, { x: 90, y: -30 }, { width: 100, height: 80 })).toEqual({ x: 70, y: 0, width: 30, height: 20 });
  });
  it("requires movement in both directions before creating a box", () => {
    expect(isBoxCreationDrag({ x: 10, y: 10 }, { x: 10, y: 10 })).toBe(false);
    expect(isBoxCreationDrag({ x: 10, y: 10 }, { x: 30, y: 11 })).toBe(false);
    expect(isBoxCreationDrag({ x: 10, y: 10 }, { x: 13, y: 13 })).toBe(true);
  });
  it("resizes from corners and edge midpoints and detects all handles", () => {
    const box = { x: 20, y: 20, width: 40, height: 30 };
    for (const handle of ["nw", "n", "ne", "e", "se", "s", "sw", "w"] as const) {
      expect(boxHandleAt(boxHandlePoint(box, handle), box, 2, 20)).toBe(handle);
    }
    expect(boxHandleAt({ x: 40, y: 0 }, box, 2, 20)).toBe("move");
    expect(boxHandlePoint(box, "w")).toEqual({ x: 20, y: 35 });
    expect(resizeBox(box, "nw", { x: 5, y: 8 }, { width: 100, height: 100 })).toEqual({ x: 5, y: 8, width: 55, height: 42 });
    expect(resizeBox(box, "e", { x: 75, y: 35 }, { width: 100, height: 100 })).toEqual({ x: 20, y: 20, width: 55, height: 30 });
  });
});
