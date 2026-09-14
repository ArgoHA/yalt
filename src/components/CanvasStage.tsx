import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import type {
  AnnotationDraft, AnnotationRecord, BboxAnnotationRecord, BboxDraft, BboxGeometry, ClassRecord,
  EditorTool, ImageRecord, PolygonAnnotationRecord, PolygonDraft, PolygonGeometry, ViewportSnapshot,
} from "../types";
import {
  BOX_RESIZE_HANDLES,
  boxFromCorners,
  boxHandleAt,
  boxHandlePoint,
  boxMoveHandlePoint,
  clampPointToImage,
  isBoxCreationDrag,
  moveBox,
  resizeBox,
  type BoxHandle,
} from "../editor/boxes";
import { hitTestTopmost, type SelectableGeometry } from "../editor/hitTest";
import { movePolygon, movePolygonVertex, polygonVertexAt, type PolygonVertex } from "../editor/polygons";
import {
  constrainViewport, fitViewport, fromSnapshot, imageToScreen, pan, restoreViewport,
  screenToImage, toSnapshot, zoomAt, type Point, type Size, type Viewport,
} from "../editor/viewport";

export interface CanvasStageHandle {
  fit: () => void;
  zoomIn: () => void;
  zoomOut: () => void;
  cancelDraft: () => boolean;
  finishDraft: () => boolean;
  removeLastDraftPoint: () => boolean;
}

interface CanvasStageProps {
  image: ImageRecord;
  imageUrl: string;
  restoredViewport: ViewportSnapshot | null;
  restoredDraft: AnnotationDraft | null;
  activeTool: EditorTool;
  activeClass: ClassRecord | null;
  appendPolygonToId: string | null;
  classes: ClassRecord[];
  annotations: AnnotationRecord[];
  selectedAnnotationId: string | null;
  onSelectAnnotation: (id: string | null) => void;
  onCreateBox: (geometry: BboxGeometry, classId: string) => void;
  onUpdateBox: (annotation: BboxAnnotationRecord, geometry: BboxGeometry) => void;
  onCreatePolygon: (geometry: PolygonGeometry, classId: string, annotationId?: string | null) => void;
  onUpdatePolygon: (annotation: PolygonAnnotationRecord, geometry: PolygonGeometry) => void;
  onDraftChange: (draft: AnnotationDraft | null) => void;
  onViewportChange: (snapshot: ViewportSnapshot, zoomPercent: number, persist: boolean) => void;
  onPointerChange: (point: Point | null) => void;
  onDecodeError: (message: string) => void;
}

type BoxDrag = { pointerId: number; annotation: BboxAnnotationRecord; origin: Point; handle: BoxHandle; preview: BboxGeometry };
type BoxCreateDrag = { pointerId: number; start: Point; startScreen: Point; classId: string };
type PolygonDrag = { pointerId: number; annotation: PolygonAnnotationRecord; origin: Point; vertex: PolygonVertex | null; preview: PolygonGeometry };
type EditHover = { annotationId: string; boxHandle?: BoxHandle; vertex?: PolygonVertex };

export const CanvasStage = forwardRef<CanvasStageHandle, CanvasStageProps>(function CanvasStage(props, forwardedRef) {
  const {
    image, imageUrl, restoredViewport, restoredDraft, activeTool, activeClass, appendPolygonToId, classes,
    annotations, selectedAnnotationId, onSelectAnnotation, onCreateBox, onUpdateBox, onCreatePolygon, onUpdatePolygon,
    onDraftChange, onViewportChange, onPointerChange, onDecodeError,
  } = props;
  const hostRef = useRef<HTMLDivElement>(null);
  const baseCanvasRef = useRef<HTMLCanvasElement>(null);
  const overlayCanvasRef = useRef<HTMLCanvasElement>(null);
  const decodedImageRef = useRef<HTMLImageElement | null>(null);
  const viewportRef = useRef<Viewport>({ scale: 1, offsetX: 0, offsetY: 0 });
  const sizeRef = useRef<Size>({ width: 0, height: 0 });
  const pointerRef = useRef<Point | null>(null);
  const panDragRef = useRef<{ pointerId: number; x: number; y: number } | null>(null);
  const boxCreateDragRef = useRef<BoxCreateDrag | null>(null);
  const boxDragRef = useRef<BoxDrag | null>(null);
  const polygonDragRef = useRef<PolygonDrag | null>(null);
  const editHoverRef = useRef<EditHover | null>(null);
  const draftRef = useRef<AnnotationDraft | null>(restoredDraft);
  const activeToolRef = useRef(activeTool);
  const activeClassRef = useRef(activeClass);
  const annotationsRef = useRef(annotations);
  const selectedIdRef = useRef(selectedAnnotationId);
  const classesRef = useRef(classes);
  const spaceDownRef = useRef(false);
  const commandDownRef = useRef(false);
  const initializedRef = useRef(false);
  const renderFrameRef = useRef<number | null>(null);
  const publishFrameRef = useRef<number | null>(null);
  const [spaceDown, setSpaceDown] = useState(false);
  const [commandDown, setCommandDown] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [editCursor, setEditCursor] = useState("default");
  const [ready, setReady] = useState(false);
  const imageSize = useCallback((): Size => ({ width: image.width, height: image.height }), [image.height, image.width]);

  const emitViewport = useCallback((viewport: Viewport, persist: boolean) => {
    const container = sizeRef.current;
    if (container.width > 0 && container.height > 0) onViewportChange(toSnapshot(viewport, container), Math.round(viewport.scale * 100), persist);
  }, [onViewportChange]);

  const drawBox = useCallback((context: CanvasRenderingContext2D, geometry: BboxGeometry, color: string, selected: boolean, ratio: number, label?: string, controls = selected, moveControl = controls) => {
    const topLeft = imageToScreen(viewportRef.current, { x: geometry.x, y: geometry.y });
    const bottomRight = imageToScreen(viewportRef.current, { x: geometry.x + geometry.width, y: geometry.y + geometry.height });
    const x = topLeft.x * ratio;
    const y = topLeft.y * ratio;
    const width = (bottomRight.x - topLeft.x) * ratio;
    const height = (bottomRight.y - topLeft.y) * ratio;
    context.strokeStyle = "rgba(0,0,0,.8)";
    context.lineWidth = (selected ? 4 : 3) * ratio;
    context.strokeRect(x, y, width, height);
    context.strokeStyle = color;
    context.lineWidth = (selected ? 2.2 : 1.5) * ratio;
    context.strokeRect(x, y, width, height);
    context.fillStyle = `${color}1f`;
    context.fillRect(x, y, width, height);
    if (label) {
      context.font = `${10 * ratio}px -apple-system, BlinkMacSystemFont, sans-serif`;
      const labelWidth = context.measureText(label).width + 10 * ratio;
      const labelHeight = 18 * ratio;
      const labelY = Math.max(0, y - labelHeight);
      context.fillStyle = color;
      context.fillRect(x, labelY, labelWidth, labelHeight);
      context.fillStyle = "#111318";
      context.fillText(label, x + 5 * ratio, labelY + 12.5 * ratio);
    }
    if (controls) {
      for (const handle of BOX_RESIZE_HANDLES) {
        drawVertex(context, imageToScreen(viewportRef.current, boxHandlePoint(geometry, handle)), color, ratio, false);
      }
      if (moveControl) {
        const topCenter = imageToScreen(viewportRef.current, boxHandlePoint(geometry, "n"));
        const movePoint = imageToScreen(viewportRef.current, boxMoveHandlePoint(geometry, 22 / viewportRef.current.scale));
        context.beginPath();
        context.moveTo(topCenter.x * ratio, topCenter.y * ratio);
        context.lineTo(movePoint.x * ratio, movePoint.y * ratio);
        context.strokeStyle = "rgba(0,0,0,.88)";
        context.lineWidth = 3 * ratio;
        context.stroke();
        context.strokeStyle = color;
        context.lineWidth = 1.25 * ratio;
        context.stroke();
        drawVertex(context, movePoint, color, ratio, false);
      }
    }
  }, []);

  const drawPolygon = useCallback((context: CanvasRenderingContext2D, geometry: PolygonGeometry, color: string, selected: boolean, ratio: number, label?: string) => {
    for (const polygon of geometry.polygons) {
      if (polygon.length === 0) continue;
      context.beginPath();
      polygon.forEach((point, index) => {
        const screen = imageToScreen(viewportRef.current, point);
        if (index === 0) context.moveTo(screen.x * ratio, screen.y * ratio);
        else context.lineTo(screen.x * ratio, screen.y * ratio);
      });
      context.closePath();
      context.fillStyle = `${color}24`;
      context.fill();
      context.strokeStyle = "rgba(0,0,0,.82)";
      context.lineWidth = (selected ? 4 : 3) * ratio;
      context.stroke();
      context.strokeStyle = color;
      context.lineWidth = (selected ? 2.2 : 1.5) * ratio;
      context.stroke();
      if (selected) {
        for (const point of polygon) drawVertex(context, imageToScreen(viewportRef.current, point), color, ratio, false);
      }
    }
    const first = geometry.polygons[0]?.[0];
    if (label && first) drawGeometryLabel(context, imageToScreen(viewportRef.current, first), label, color, ratio);
  }, []);

  const renderNow = useCallback(() => {
    const baseCanvas = baseCanvasRef.current;
    const overlayCanvas = overlayCanvasRef.current;
    const decodedImage = decodedImageRef.current;
    if (!baseCanvas || !overlayCanvas || !decodedImage) return;
    const size = sizeRef.current;
    const ratio = window.devicePixelRatio || 1;
    const pixelWidth = Math.max(1, Math.round(size.width * ratio));
    const pixelHeight = Math.max(1, Math.round(size.height * ratio));
    for (const canvas of [baseCanvas, overlayCanvas]) {
      if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) { canvas.width = pixelWidth; canvas.height = pixelHeight; }
    }
    const viewport = viewportRef.current;
    const base = baseCanvas.getContext("2d");
    const overlay = overlayCanvas.getContext("2d");
    if (!base || !overlay) return;
    base.setTransform(1, 0, 0, 1, 0, 0);
    base.clearRect(0, 0, pixelWidth, pixelHeight);
    base.imageSmoothingEnabled = true;
    base.imageSmoothingQuality = "high";
    base.setTransform(ratio * viewport.scale, 0, 0, ratio * viewport.scale, ratio * viewport.offsetX, ratio * viewport.offsetY);
    base.drawImage(decodedImage, 0, 0, image.width, image.height);
    overlay.setTransform(1, 0, 0, 1, 0, 0);
    overlay.clearRect(0, 0, pixelWidth, pixelHeight);
    const topLeft = imageToScreen(viewport, { x: 0, y: 0 });
    const bottomRight = imageToScreen(viewport, { x: image.width, y: image.height });
    overlay.strokeStyle = "rgba(255,255,255,.32)";
    overlay.lineWidth = ratio;
    overlay.strokeRect(Math.round(topLeft.x * ratio) + 0.5, Math.round(topLeft.y * ratio) + 0.5, Math.round((bottomRight.x - topLeft.x) * ratio), Math.round((bottomRight.y - topLeft.y) * ratio));

    const colorMap = new Map(classesRef.current.map((item) => [item.id, item]));
    const hovered = editHoverRef.current;
    for (const annotation of annotationsRef.current) {
      if (!annotation.isVisible || annotation.id === selectedIdRef.current) continue;
      const record = colorMap.get(annotation.classId);
      if (annotation.kind === "bbox") {
        const geometry = boxDragRef.current?.annotation.id === annotation.id ? boxDragRef.current.preview : annotation.geometry;
        drawBox(overlay, geometry, record?.color ?? "#fff", false, ratio, record?.name, hovered?.annotationId === annotation.id);
      }
      else drawPolygon(overlay, annotation.geometry, record?.color ?? "#fff", false, ratio, record?.name);
    }
    const selected = annotationsRef.current.find((item) => item.id === selectedIdRef.current && item.isVisible);
    if (selected) {
      const record = colorMap.get(selected.classId);
      if (selected.kind === "bbox") {
        const geometry = boxDragRef.current?.annotation.id === selected.id ? boxDragRef.current.preview : selected.geometry;
        const showControls = !hovered || hovered.annotationId === selected.id;
        drawBox(overlay, geometry, record?.color ?? "#fff", true, ratio, record?.name, showControls);
      } else {
        const geometry = polygonDragRef.current?.annotation.id === selected.id ? polygonDragRef.current.preview : selected.geometry;
        drawPolygon(overlay, geometry, record?.color ?? "#fff", true, ratio, record?.name);
      }
    }
    if ((hovered?.boxHandle || hovered?.vertex) && (activeToolRef.current === "select" || activeToolRef.current === "rectangle")) {
      const annotation = annotationsRef.current.find((item) => item.id === hovered.annotationId && item.isVisible);
      if (annotation) {
        const record = colorMap.get(annotation.classId);
        if (annotation.kind === "bbox" && hovered.boxHandle) {
          const geometry = boxDragRef.current?.annotation.id === annotation.id ? boxDragRef.current.preview : annotation.geometry;
          const point = hovered.boxHandle === "move"
            ? boxMoveHandlePoint(geometry, 22 / viewport.scale)
            : boxHandlePoint(geometry, hovered.boxHandle);
          drawVertex(overlay, imageToScreen(viewport, point), record?.color ?? "#fff", ratio, true);
        } else if (annotation.kind === "polygon" && hovered.vertex) {
          const geometry = polygonDragRef.current?.annotation.id === annotation.id ? polygonDragRef.current.preview : annotation.geometry;
          const point = geometry.polygons[hovered.vertex.polygonIndex]?.[hovered.vertex.pointIndex];
          if (point) drawVertex(overlay, imageToScreen(viewport, point), record?.color ?? "#fff", ratio, true);
        }
      }
    }
    const pointer = pointerRef.current;
    const draft = draftRef.current;
    if (draft && "x" in draft && pointer) {
      const current = clampPointToImage(screenToImage(viewport, pointer), imageSize());
      const record = colorMap.get(draft.classId);
      drawBox(overlay, boxFromCorners({ x: draft.x, y: draft.y }, current), record?.color ?? "#fff", false, ratio, record?.name, true, false);
    } else if (draft && "points" in draft) {
      const record = colorMap.get(draft.classId);
      const color = record?.color ?? "#fff";
      const preview = pointer ? clampPointToImage(screenToImage(viewport, pointer), imageSize()) : null;
      drawPolygonDraft(overlay, draft, preview, color, ratio, viewport);
    }
    const hoveringEditableBox = activeToolRef.current === "rectangle" && hovered && !commandDownRef.current;
    if (pointer && activeToolRef.current !== "select" && !hoveringEditableBox && !spaceDownRef.current && !panDragRef.current) {
      const x = Math.round(pointer.x * ratio) + 0.5;
      const y = Math.round(pointer.y * ratio) + 0.5;
      const gap = 6 * ratio;
      const color = activeClassRef.current?.color ?? "#f2f4f5";
      const crosshair = (stroke: string, width: number) => {
        overlay.beginPath();
        overlay.moveTo(0, y); overlay.lineTo(x - gap, y); overlay.moveTo(x + gap, y); overlay.lineTo(pixelWidth, y);
        overlay.moveTo(x, 0); overlay.lineTo(x, y - gap); overlay.moveTo(x, y + gap); overlay.lineTo(x, pixelHeight);
        overlay.strokeStyle = stroke; overlay.lineWidth = width * ratio; overlay.stroke();
      };
      crosshair("rgba(0,0,0,.84)", 2.6);
      crosshair(color, 1);
    }
  }, [drawBox, drawPolygon, image.height, image.width, imageSize]);

  const render = useCallback(() => {
    if (renderFrameRef.current !== null) return;
    renderFrameRef.current = window.requestAnimationFrame(() => { renderFrameRef.current = null; renderNow(); });
  }, [renderNow]);
  const publishViewport = useCallback(() => {
    if (publishFrameRef.current !== null) return;
    publishFrameRef.current = window.requestAnimationFrame(() => { publishFrameRef.current = null; emitViewport(viewportRef.current, true); });
  }, [emitViewport]);

  useEffect(() => () => {
    if (renderFrameRef.current !== null) window.cancelAnimationFrame(renderFrameRef.current);
    if (publishFrameRef.current !== null) window.cancelAnimationFrame(publishFrameRef.current);
  }, []);
  useEffect(() => {
    activeToolRef.current = activeTool;
    editHoverRef.current = null;
    setEditCursor("default");
    render();
  }, [activeTool, render]);
  useEffect(() => { activeClassRef.current = activeClass; render(); }, [activeClass, render]);
  useEffect(() => { annotationsRef.current = annotations; render(); }, [annotations, render]);
  useEffect(() => { selectedIdRef.current = selectedAnnotationId; render(); }, [selectedAnnotationId, render]);
  useEffect(() => { classesRef.current = classes; render(); }, [classes, render]);
  useEffect(() => { spaceDownRef.current = spaceDown; render(); }, [render, spaceDown]);
  useEffect(() => { draftRef.current = restoredDraft; render(); }, [restoredDraft, render]);

  const updateViewport = useCallback((next: Viewport) => {
    viewportRef.current = constrainViewport(next, sizeRef.current, imageSize());
    render(); publishViewport();
  }, [imageSize, publishViewport, render]);
  const fit = useCallback(() => { viewportRef.current = fitViewport(sizeRef.current, imageSize()); render(); publishViewport(); }, [imageSize, publishViewport, render]);
  const zoomFromCenter = useCallback((factor: number) => {
    const size = sizeRef.current;
    updateViewport(zoomAt(viewportRef.current, { x: size.width / 2, y: size.height / 2 }, factor));
  }, [updateViewport]);
  const cancelDraft = useCallback(() => {
    if (!draftRef.current) return false;
    boxCreateDragRef.current = null;
    draftRef.current = null; onDraftChange(null); render();
    return true;
  }, [onDraftChange, render]);
  const finishDraft = useCallback(() => {
    const draft = draftRef.current;
    if (!draft || !("points" in draft)) return false;
    if (draft.points.length < 3) return true;
    onCreatePolygon({ polygons: [draft.points] }, draft.classId, draft.annotationId);
    draftRef.current = null; onDraftChange(null); render();
    return true;
  }, [onCreatePolygon, onDraftChange, render]);
  const removeLastDraftPoint = useCallback(() => {
    const draft = draftRef.current;
    if (!draft || !("points" in draft)) return false;
    const points = draft.points.slice(0, -1);
    const next: PolygonDraft | null = points.length > 0 ? { ...draft, points } : null;
    draftRef.current = next; onDraftChange(next); render();
    return true;
  }, [onDraftChange, render]);
  useImperativeHandle(forwardedRef, () => ({
    fit,
    zoomIn: () => zoomFromCenter(1.2),
    zoomOut: () => zoomFromCenter(1 / 1.2),
    cancelDraft,
    finishDraft,
    removeLastDraftPoint,
  }), [cancelDraft, finishDraft, fit, removeLastDraftPoint, zoomFromCenter]);

  const initializeViewport = useCallback(() => {
    const size = sizeRef.current;
    if (initializedRef.current || !decodedImageRef.current || size.width <= 0 || size.height <= 0) return;
    viewportRef.current = restoreViewport(restoredViewport, size, imageSize());
    initializedRef.current = true; setReady(true); render(); emitViewport(viewportRef.current, false);
  }, [emitViewport, imageSize, render, restoredViewport]);
  useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const initialBounds = host.getBoundingClientRect();
    if (initialBounds.width > 0 && initialBounds.height > 0) sizeRef.current = { width: initialBounds.width, height: initialBounds.height };
    const observer = new ResizeObserver(([entry]) => {
      const previousSize = sizeRef.current;
      const nextSize = { width: entry.contentRect.width, height: entry.contentRect.height };
      if (nextSize.width <= 0 || nextSize.height <= 0) return;
      const previousViewport = viewportRef.current;
      sizeRef.current = nextSize;
      if (!initializedRef.current) initializeViewport();
      else if (previousSize.width > 0 && previousSize.height > 0) viewportRef.current = constrainViewport(fromSnapshot(toSnapshot(previousViewport, previousSize), nextSize), nextSize, imageSize());
      render();
    });
    observer.observe(host);
    return () => observer.disconnect();
  }, [imageSize, initializeViewport, render]);
  useEffect(() => {
    let cancelled = false;
    initializedRef.current = false; decodedImageRef.current = null; setReady(false);
    const decoded = new Image(); decoded.decoding = "async"; decoded.src = imageUrl;
    decoded.decode().then(() => { if (!cancelled) { decodedImageRef.current = decoded; initializeViewport(); } }).catch(() => {
      if (!cancelled) { setReady(false); onDecodeError(`Could not decode ${image.fileName}. The file may be damaged or use an unsupported JPEG/PNG encoding.`); }
    });
    return () => { cancelled = true; initializedRef.current = false; decodedImageRef.current = null; };
  }, [image.fileName, imageUrl, initializeViewport, onDecodeError]);
  useEffect(() => {
    const canvas = overlayCanvasRef.current;
    if (!canvas) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = canvas.getBoundingClientRect();
      if (event.ctrlKey || event.metaKey) updateViewport(zoomAt(viewportRef.current, { x: event.clientX - rect.left, y: event.clientY - rect.top }, Math.exp(-event.deltaY * 0.012)));
      else updateViewport(pan(viewportRef.current, { x: -event.deltaX, y: -event.deltaY }));
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [updateViewport]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code === "Space" && !isTypingTarget(event.target)) { event.preventDefault(); setSpaceDown(true); }
      if (event.key === "Meta") {
        commandDownRef.current = true;
        setCommandDown(true);
        if (activeToolRef.current === "rectangle") {
          editHoverRef.current = null;
          setEditCursor("crosshair");
        }
        render();
      }
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.code === "Space") setSpaceDown(false);
      if (event.key === "Meta") {
        commandDownRef.current = false;
        setCommandDown(false);
        if (activeToolRef.current === "rectangle") setEditCursor("default");
        render();
      }
    };
    const onBlur = () => {
      setSpaceDown(false);
      commandDownRef.current = false;
      setCommandDown(false);
      if (activeToolRef.current === "rectangle") setEditCursor("default");
    };
    window.addEventListener("keydown", onKeyDown); window.addEventListener("keyup", onKeyUp); window.addEventListener("blur", onBlur);
    return () => { window.removeEventListener("keydown", onKeyDown); window.removeEventListener("keyup", onKeyUp); window.removeEventListener("blur", onBlur); };
  }, [render]);

  const pointerCoordinates = (event: React.PointerEvent<HTMLCanvasElement>): Point => {
    const rect = event.currentTarget.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };
  const reportPointer = (screenPoint: Point | null) => {
    pointerRef.current = screenPoint;
    if (!screenPoint) onPointerChange(null);
    else {
      const point = screenToImage(viewportRef.current, screenPoint);
      onPointerChange(point.x >= 0 && point.y >= 0 && point.x <= image.width && point.y <= image.height ? point : null);
    }
    render();
  };
  const imagePoint = (screen: Point) => clampPointToImage(screenToImage(viewportRef.current, screen), imageSize());
  const isInsideImage = (screen: Point) => {
    const point = screenToImage(viewportRef.current, screen);
    return point.x >= 0 && point.y >= 0 && point.x <= image.width && point.y <= image.height;
  };
  const findEditTarget = (point: Point): EditHover | null => {
    const visible = annotationsRef.current.filter((item) => item.isVisible);
    for (let index = visible.length - 1; index >= 0; index -= 1) {
      const annotation = visible[index];
      if (annotation.kind === "bbox") {
        const boxHandle = boxHandleAt(point, annotation.geometry, 8 / viewportRef.current.scale, 22 / viewportRef.current.scale);
        if (boxHandle) return { annotationId: annotation.id, boxHandle };
      } else {
        const vertex = polygonVertexAt(point, annotation.geometry, 8 / viewportRef.current.scale);
        if (vertex) return { annotationId: annotation.id, vertex };
      }
    }
    const geometries: SelectableGeometry[] = [];
    for (const item of visible) {
      if (item.kind === "bbox") geometries.push({ id: item.id, kind: "bbox", rect: item.geometry });
      else for (const points of item.geometry.polygons) geometries.push({ id: item.id, kind: "polygon", points });
    }
    const hitId = hitTestTopmost(point, geometries, 3 / viewportRef.current.scale);
    return hitId ? { annotationId: hitId } : null;
  };
  const updateEditHover = (screen: Point | null) => {
    const editingTool = activeToolRef.current === "select" || activeToolRef.current === "rectangle";
    const bypassingBoxes = activeToolRef.current === "rectangle" && commandDownRef.current;
    const next = editingTool && !bypassingBoxes && screen
      ? findEditTarget(screenToImage(viewportRef.current, screen))
      : null;
    const previous = editHoverRef.current;
    if (hoverKey(previous) !== hoverKey(next)) {
      editHoverRef.current = next;
      setEditCursor(next?.boxHandle ? boxCursor(next.boxHandle) : next?.vertex ? "pointer" : next ? "pointer" : "default");
      render();
    }
  };
  const cursor = dragging || spaceDown
    ? dragging ? "grabbing" : "grab"
    : activeTool === "rectangle" && commandDown ? "crosshair"
      : activeTool === "select" || activeTool === "rectangle" ? editCursor
        : "none";

  return (
    <div className="canvas-stage" ref={hostRef} data-ready={ready}>
      <canvas className="image-canvas" ref={baseCanvasRef} aria-hidden="true" />
      <canvas
        className="interaction-canvas" ref={overlayCanvasRef} tabIndex={0}
        aria-label={`${image.fileName} annotation canvas`} style={{ cursor }}
        onPointerDown={(event) => {
          const screen = pointerCoordinates(event);
          if (spaceDown || event.button === 1) {
            event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId);
            panDragRef.current = { pointerId: event.pointerId, x: screen.x, y: screen.y }; setDragging(true); render(); return;
          }
          if (event.button !== 0) return;
          const insideImage = isInsideImage(screen);
          const point = insideImage ? imagePoint(screen) : screenToImage(viewportRef.current, screen);
          if (activeToolRef.current === "rectangle") {
            const target = event.metaKey ? null : findEditTarget(point);
            const hit = annotationsRef.current.find((item) => item.id === target?.annotationId);
            if (hit?.kind === "bbox") {
              onSelectAnnotation(hit.id);
              if (target?.boxHandle) {
                event.currentTarget.setPointerCapture(event.pointerId);
                boxDragRef.current = {
                  pointerId: event.pointerId,
                  annotation: hit,
                  origin: point,
                  handle: target.boxHandle,
                  preview: hit.geometry,
                };
                setEditCursor(boxCursor(target.boxHandle));
                render();
              }
              return;
            }
            if (!insideImage) return;
            onSelectAnnotation(null);
            if (!activeClassRef.current) return;
            event.currentTarget.setPointerCapture(event.pointerId);
            const draft: BboxDraft = { imageId: image.id, classId: activeClassRef.current.id, x: point.x, y: point.y };
            boxCreateDragRef.current = { pointerId: event.pointerId, start: point, startScreen: screen, classId: draft.classId };
            draftRef.current = draft;
            onDraftChange(draft);
            reportPointer(screen);
            return;
          }
          if (activeToolRef.current === "polygon") {
            if (!insideImage) return;
            if (!activeClassRef.current && !draftRef.current) return;
            const existing = draftRef.current && "points" in draftRef.current ? draftRef.current : null;
            if (existing && existing.points.length >= 3 && Math.hypot(existing.points[0].x - point.x, existing.points[0].y - point.y) <= 10 / viewportRef.current.scale) {
              finishDraft();
            } else {
              const draft: PolygonDraft = existing
                ? { ...existing, points: [...existing.points, point] }
                : { imageId: image.id, classId: activeClassRef.current!.id, points: [point], annotationId: appendPolygonToId };
              draftRef.current = draft; onDraftChange(draft); render();
            }
            return;
          }
          if (activeToolRef.current === "select") {
            const target = findEditTarget(point);
            onSelectAnnotation(target?.annotationId ?? null);
            const hit = annotationsRef.current.find((item) => item.id === target?.annotationId);
            if (hit?.kind === "bbox" && target?.boxHandle) {
              event.currentTarget.setPointerCapture(event.pointerId);
              boxDragRef.current = { pointerId: event.pointerId, annotation: hit, origin: point, handle: target.boxHandle, preview: hit.geometry };
              setEditCursor(boxCursor(target.boxHandle));
              render();
            } else if (hit?.kind === "polygon") {
              event.currentTarget.setPointerCapture(event.pointerId);
              polygonDragRef.current = { pointerId: event.pointerId, annotation: hit, origin: point, vertex: target?.vertex ?? null, preview: hit.geometry };
              setEditCursor(target?.vertex ? "pointer" : "move");
              render();
            }
          }
        }}
        onPointerMove={(event) => {
          const screen = pointerCoordinates(event);
          if (commandDownRef.current !== event.metaKey) {
            commandDownRef.current = event.metaKey;
            setCommandDown(event.metaKey);
            if (event.metaKey && activeToolRef.current === "rectangle") editHoverRef.current = null;
          }
          const panDrag = panDragRef.current;
          if (panDrag?.pointerId === event.pointerId) {
            updateViewport(pan(viewportRef.current, { x: screen.x - panDrag.x, y: screen.y - panDrag.y })); panDrag.x = screen.x; panDrag.y = screen.y;
          }
          const boxCreateDrag = boxCreateDragRef.current;
          const boxDrag = boxDragRef.current;
          if (boxCreateDrag?.pointerId === event.pointerId) {
            render();
          } else if (boxDrag?.pointerId === event.pointerId) {
            const point = boxDrag.handle === "move"
              ? screenToImage(viewportRef.current, screen)
              : imagePoint(screen);
            boxDrag.preview = boxDrag.handle === "move"
              ? moveBox(boxDrag.annotation.geometry, { x: point.x - boxDrag.origin.x, y: point.y - boxDrag.origin.y }, imageSize())
              : resizeBox(boxDrag.annotation.geometry, boxDrag.handle, point, imageSize());
            render();
          } else if (polygonDragRef.current?.pointerId === event.pointerId) {
            const drag = polygonDragRef.current;
            const point = imagePoint(screen);
            drag.preview = drag.vertex
              ? movePolygonVertex(drag.annotation.geometry, drag.vertex, point, imageSize())
              : movePolygon(drag.annotation.geometry, { x: point.x - drag.origin.x, y: point.y - drag.origin.y }, imageSize());
            render();
          } else if (!panDrag) {
            updateEditHover(screen);
          }
          reportPointer(screen);
        }}
        onPointerUp={(event) => {
          if (panDragRef.current?.pointerId === event.pointerId) { panDragRef.current = null; setDragging(false); }
          if (boxCreateDragRef.current?.pointerId === event.pointerId) {
            const drag = boxCreateDragRef.current;
            const screen = pointerCoordinates(event);
            const end = imagePoint(screen);
            const geometry = boxFromCorners(drag.start, end);
            boxCreateDragRef.current = null;
            draftRef.current = null;
            onDraftChange(null);
            if (isBoxCreationDrag(drag.startScreen, screen) && geometry.width >= 1 && geometry.height >= 1) {
              onCreateBox(geometry, drag.classId);
            }
            render();
          }
          if (boxDragRef.current?.pointerId === event.pointerId) {
            const drag = boxDragRef.current; boxDragRef.current = null;
            if (JSON.stringify(drag.preview) !== JSON.stringify(drag.annotation.geometry)) onUpdateBox(drag.annotation, drag.preview);
            render();
          }
          if (polygonDragRef.current?.pointerId === event.pointerId) {
            const drag = polygonDragRef.current; polygonDragRef.current = null;
            if (JSON.stringify(drag.preview) !== JSON.stringify(drag.annotation.geometry)) onUpdatePolygon(drag.annotation, drag.preview);
            render();
          }
          const screen = pointerCoordinates(event);
          updateEditHover(screen);
          reportPointer(screen);
        }}
        onPointerCancel={() => {
          if (boxCreateDragRef.current) {
            boxCreateDragRef.current = null;
            draftRef.current = null;
            onDraftChange(null);
          }
          panDragRef.current = null;
          boxDragRef.current = null;
          polygonDragRef.current = null;
          editHoverRef.current = null;
          setEditCursor("default");
          setDragging(false);
          render();
        }}
        onPointerLeave={() => { if (!panDragRef.current && !boxCreateDragRef.current && !boxDragRef.current && !polygonDragRef.current) { editHoverRef.current = null; setEditCursor("default"); reportPointer(null); } }}
      />
      {!ready && <div className="canvas-loading"><span />Decoding image…</div>}
    </div>
  );
});

function isTypingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable);
}

function boxCursor(handle: BoxHandle): string {
  if (handle === "move") return "grab";
  if (handle === "n" || handle === "s") return "ns-resize";
  if (handle === "e" || handle === "w") return "ew-resize";
  return handle === "nw" || handle === "se" ? "nwse-resize" : "nesw-resize";
}

function hoverKey(hover: EditHover | null): string {
  if (!hover) return "";
  if (hover.boxHandle) return `${hover.annotationId}:box:${hover.boxHandle}`;
  if (hover.vertex) return `${hover.annotationId}:point:${hover.vertex.polygonIndex}:${hover.vertex.pointIndex}`;
  return `${hover.annotationId}:move`;
}

function drawVertex(
  context: CanvasRenderingContext2D,
  point: Point,
  color: string,
  ratio: number,
  target: boolean,
) {
  context.beginPath();
  context.arc(point.x * ratio, point.y * ratio, (target ? 7 : 4) * ratio, 0, Math.PI * 2);
  context.fillStyle = target ? "rgba(18,20,24,.88)" : "#f7f7fa";
  context.fill();
  context.strokeStyle = target ? "rgba(0,0,0,.9)" : "#14161a";
  context.lineWidth = (target ? 4 : 1) * ratio;
  context.stroke();
  context.beginPath();
  context.arc(point.x * ratio, point.y * ratio, (target ? 6 : 3.5) * ratio, 0, Math.PI * 2);
  context.strokeStyle = color;
  context.lineWidth = (target ? 2 : 1.5) * ratio;
  context.stroke();
}

function drawGeometryLabel(
  context: CanvasRenderingContext2D,
  point: Point,
  label: string,
  color: string,
  ratio: number,
) {
  context.font = `${10 * ratio}px -apple-system, BlinkMacSystemFont, sans-serif`;
  const width = context.measureText(label).width + 10 * ratio;
  const height = 18 * ratio;
  const x = point.x * ratio;
  const y = Math.max(0, point.y * ratio - height);
  context.fillStyle = color;
  context.fillRect(x, y, width, height);
  context.fillStyle = "#111318";
  context.fillText(label, x + 5 * ratio, y + 12.5 * ratio);
}

function drawPolygonDraft(
  context: CanvasRenderingContext2D,
  draft: PolygonDraft,
  preview: Point | null,
  color: string,
  ratio: number,
  viewport: Viewport,
) {
  const points = draft.points.map((point) => imageToScreen(viewport, point));
  if (points.length === 0) return;
  context.beginPath();
  context.moveTo(points[0].x * ratio, points[0].y * ratio);
  for (const point of points.slice(1)) context.lineTo(point.x * ratio, point.y * ratio);
  if (preview) {
    const screen = imageToScreen(viewport, preview);
    context.lineTo(screen.x * ratio, screen.y * ratio);
  }
  context.strokeStyle = "rgba(0,0,0,.85)";
  context.lineWidth = 3 * ratio;
  context.stroke();
  context.strokeStyle = color;
  context.lineWidth = 1.5 * ratio;
  context.stroke();
  for (const point of points) drawVertex(context, point, color, ratio, false);
  const nearFirst = preview && draft.points.length >= 3
    && Math.hypot(preview.x - draft.points[0].x, preview.y - draft.points[0].y) <= 10 / viewport.scale;
  drawVertex(context, points[0], color, ratio, Boolean(nearFirst));
}
