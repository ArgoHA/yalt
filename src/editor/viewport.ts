import type { ViewportSnapshot } from "../types";

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Viewport {
  scale: number;
  offsetX: number;
  offsetY: number;
}

export const MIN_SCALE = 0.01;
export const MAX_SCALE = 64;

export function fitViewport(container: Size, image: Size, padding = 42): Viewport {
  if (container.width <= 0 || container.height <= 0 || image.width <= 0 || image.height <= 0) {
    return { scale: 1, offsetX: 0, offsetY: 0 };
  }
  const availableWidth = Math.max(1, container.width - padding * 2);
  const availableHeight = Math.max(1, container.height - padding * 2);
  const scale = Math.min(availableWidth / image.width, availableHeight / image.height, 1);
  return {
    scale,
    offsetX: (container.width - image.width * scale) / 2,
    offsetY: (container.height - image.height * scale) / 2,
  };
}

export function imageToScreen(viewport: Viewport, point: Point): Point {
  return {
    x: point.x * viewport.scale + viewport.offsetX,
    y: point.y * viewport.scale + viewport.offsetY,
  };
}

export function screenToImage(viewport: Viewport, point: Point): Point {
  return {
    x: (point.x - viewport.offsetX) / viewport.scale,
    y: (point.y - viewport.offsetY) / viewport.scale,
  };
}

export function zoomAt(viewport: Viewport, screenPoint: Point, factor: number): Viewport {
  const anchor = screenToImage(viewport, screenPoint);
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, viewport.scale * factor));
  return {
    scale,
    offsetX: screenPoint.x - anchor.x * scale,
    offsetY: screenPoint.y - anchor.y * scale,
  };
}

export function pan(viewport: Viewport, delta: Point): Viewport {
  return {
    ...viewport,
    offsetX: viewport.offsetX + delta.x,
    offsetY: viewport.offsetY + delta.y,
  };
}

export function constrainViewport(
  viewport: Viewport,
  container: Size,
  image: Size,
  minimumVisible = 72,
): Viewport {
  const constrainAxis = (offset: number, scaled: number, available: number) => {
    if (scaled <= available) return (available - scaled) / 2;
    return Math.min(available - minimumVisible, Math.max(minimumVisible - scaled, offset));
  };
  return {
    ...viewport,
    offsetX: constrainAxis(viewport.offsetX, image.width * viewport.scale, container.width),
    offsetY: constrainAxis(viewport.offsetY, image.height * viewport.scale, container.height),
  };
}

export function toSnapshot(viewport: Viewport, container: Size): ViewportSnapshot {
  const center = screenToImage(viewport, { x: container.width / 2, y: container.height / 2 });
  return { scale: viewport.scale, centerX: center.x, centerY: center.y };
}

export function fromSnapshot(snapshot: ViewportSnapshot, container: Size): Viewport {
  return {
    scale: snapshot.scale,
    offsetX: container.width / 2 - snapshot.centerX * snapshot.scale,
    offsetY: container.height / 2 - snapshot.centerY * snapshot.scale,
  };
}

export function restoreViewport(
  snapshot: ViewportSnapshot | null,
  container: Size,
  image: Size,
): Viewport {
  if (!snapshot
    || !Number.isFinite(snapshot.scale)
    || !Number.isFinite(snapshot.centerX)
    || !Number.isFinite(snapshot.centerY)
    || snapshot.scale < MIN_SCALE
    || snapshot.scale > MAX_SCALE) {
    return fitViewport(container, image);
  }

  // A Phase 2 initialization race could persist a center calculated while the
  // canvas was still 0 × 0. Keep valid zoom state while repairing that center.
  const repaired = {
    scale: snapshot.scale,
    centerX: Math.min(image.width, Math.max(0, snapshot.centerX)),
    centerY: Math.min(image.height, Math.max(0, snapshot.centerY)),
  };
  return constrainViewport(fromSnapshot(repaired, container), container, image);
}
