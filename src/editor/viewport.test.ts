import { describe, expect, it } from "vitest";
import {
  fitViewport,
  fromSnapshot,
  imageToScreen,
  restoreViewport,
  screenToImage,
  toSnapshot,
  zoomAt,
} from "./viewport";

describe("viewport transforms", () => {
  it("round-trips between image and screen coordinates", () => {
    const viewport = { scale: 2.5, offsetX: -18, offsetY: 42 };
    const imagePoint = { x: 92.25, y: 17.5 };
    const result = screenToImage(viewport, imageToScreen(viewport, imagePoint));
    expect(result.x).toBeCloseTo(imagePoint.x);
    expect(result.y).toBeCloseTo(imagePoint.y);
  });

  it("keeps the pointer anchor stationary while zooming", () => {
    const viewport = { scale: 1.2, offsetX: 30, offsetY: -12 };
    const pointer = { x: 420, y: 260 };
    const imagePoint = screenToImage(viewport, pointer);
    const zoomed = zoomAt(viewport, pointer, 1.8);
    expect(imageToScreen(zoomed, imagePoint).x).toBeCloseTo(pointer.x);
    expect(imageToScreen(zoomed, imagePoint).y).toBeCloseTo(pointer.y);
  });

  it("restores the same image center at a new window size", () => {
    const originalSize = { width: 900, height: 600 };
    const viewport = fitViewport(originalSize, { width: 1920, height: 1080 });
    const snapshot = toSnapshot(viewport, originalSize);
    const restored = fromSnapshot(snapshot, { width: 1200, height: 800 });
    const center = screenToImage(restored, { x: 600, y: 400 });
    expect(center.x).toBeCloseTo(snapshot.centerX);
    expect(center.y).toBeCloseTo(snapshot.centerY);
  });

  it("repairs a viewport center persisted before canvas layout", () => {
    const container = { width: 1200, height: 800 };
    const image = { width: 1920, height: 1080 };
    const restored = restoreViewport(
      { scale: 0.5, centerX: 4100, centerY: -900 },
      container,
      image,
    );
    const center = screenToImage(restored, { x: container.width / 2, y: container.height / 2 });
    expect(center.x).toBeLessThanOrEqual(image.width);
    expect(center.y).toBeGreaterThanOrEqual(0);
  });

  it("fits instead of restoring non-finite viewport data", () => {
    const container = { width: 900, height: 600 };
    const image = { width: 1920, height: 1080 };
    expect(restoreViewport(
      { scale: Number.NaN, centerX: 0, centerY: 0 },
      container,
      image,
    )).toEqual(fitViewport(container, image));
  });
});
