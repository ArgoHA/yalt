// @vitest-environment happy-dom
import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { KeypointAnnotationRecord } from "../types";
import { UNLABELED } from "../editor/keypoints";
import { CanvasStage, type CanvasStageHandle } from "./CanvasStage";

describe("canvas pointer", () => {
  let host: HTMLDivElement;
  let root: Root;
  let frames: FrameRequestCallback[];
  let finishDecode: () => void;
  const context = {
    setTransform: vi.fn(), clearRect: vi.fn(), drawImage: vi.fn(), strokeRect: vi.fn(),
    beginPath: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(), arc: vi.fn(), fill: vi.fn(),
  };
  const stage = createRef<CanvasStageHandle>();

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    frames = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => frames.push(callback));
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context as unknown as CanvasRenderingContext2D);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 500, bottom: 400, width: 500, height: 400, toJSON: () => ({}),
    });
    vi.stubGlobal("Image", class {
      decode() { return new Promise<void>((resolve) => { finishDecode = resolve; }); }
    });
    HTMLElement.prototype.setPointerCapture ??= () => undefined;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(() => root.unmount());
    host.remove();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function flushFrames() {
    const pending = frames;
    frames = [];
    pending.forEach((callback) => callback(0));
  }

  async function mount(tool: "rectangle" | "polygon" = "rectangle") {
    await act(async () => {
      root.render(<CanvasStage ref={stage}
        image={{ id: "image", fileName: "image.jpg", relativePath: "image.jpg", width: 100, height: 100, orientation: 1, status: "active" }}
        imageUrl="blob:image" restoredViewport={null} restoredDraft={null} activeTool={tool}
        activeClass={null} appendPolygonToId={null} classes={[]} annotations={[]} selectedAnnotationId={null}
        onSelectAnnotation={vi.fn()} onCreateBox={vi.fn()} onUpdateBox={vi.fn()} onCreatePolygon={vi.fn()}
        onUpdatePolygon={vi.fn()} onDraftChange={vi.fn()} onViewportChange={vi.fn()} onPointerChange={vi.fn()} onDecodeError={vi.fn()}
      />);
    });
    return host.querySelector<HTMLCanvasElement>(".interaction-canvas")!;
  }

  it.each(["rectangle", "polygon"] as const)("draws the %s cursor in the gray margin and on the image", async (tool) => {
    const canvas = await mount(tool);
    await act(async () => finishDecode());
    flushFrames();
    expect(canvas.style.cursor).toBe("none");
    for (const [x, y] of [[10, 10], [250, 200]]) {
      context.arc.mockClear();
      await act(() => { canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: x, clientY: y })); });
      flushFrames();
      expect(context.arc).toHaveBeenCalledWith(x, y, 2.25, 0, Math.PI * 2);
    }
  });

  it("places keypoints on click and starts the next object's box on drag", async () => {
    const pose: KeypointAnnotationRecord = {
      id: "pose", imageId: "image", classId: "person", kind: "keypoints", isVisible: true, createdAtMs: 0, updatedAtMs: 0,
      geometry: { bbox: { x: 5, y: 5, width: 80, height: 80 }, keypoints: [UNLABELED, UNLABELED] },
    };
    const person = { id: "person", name: "person", position: 0, color: "#ff0000", shortcut: "1" };
    const onUpdateKeypoints = vi.fn();
    const onKeypointPlacementChange = vi.fn();
    const onCreateBox = vi.fn();
    const renderStage = (index: number) => root.render(<CanvasStage ref={stage}
      image={{ id: "image", fileName: "image.jpg", relativePath: "image.jpg", width: 100, height: 100, orientation: 1, status: "active" }}
      imageUrl="blob:image" restoredViewport={null} restoredDraft={null} activeTool="rectangle"
      activeClass={person} appendPolygonToId={null} classes={[person]} annotations={[pose]} selectedAnnotationId="pose"
      keypointSchema={{ names: ["head", "tail"], skeleton: [[0, 1]] }} keypointPlacement={{ annotationId: "pose", index }}
      onUpdateKeypoints={onUpdateKeypoints} onKeypointPlacementChange={onKeypointPlacementChange}
      onSelectAnnotation={vi.fn()} onCreateBox={onCreateBox} onUpdateBox={vi.fn()} onCreatePolygon={vi.fn()}
      onUpdatePolygon={vi.fn()} onDraftChange={vi.fn()} onViewportChange={vi.fn()} onPointerChange={vi.fn()} onDecodeError={vi.fn()}
    />);
    await act(async () => renderStage(0));
    await act(async () => finishDecode());
    const canvas = host.querySelector<HTMLCanvasElement>(".interaction-canvas")!;
    const pointer = async (type: string, x: number, y: number, shiftKey = false) => {
      await act(() => { canvas.dispatchEvent(new PointerEvent(type, { bubbles: true, clientX: x, clientY: y, button: 0, pointerId: 1, shiftKey })); });
    };

    // A 100 × 100 image fits at 1× in the 500 × 400 stage, offset by (200, 150).
    await pointer("pointerdown", 220, 170);
    await pointer("pointerup", 220, 170);
    expect(onUpdateKeypoints).toHaveBeenLastCalledWith(pose, { ...pose.geometry, keypoints: [{ x: 20, y: 20, v: 2 }, UNLABELED] });
    expect(onKeypointPlacementChange).toHaveBeenLastCalledWith({ annotationId: "pose", index: 1 });

    // Shift places the next point as occluded and finishes the object.
    await pointer("pointerdown", 240, 190, true);
    await pointer("pointerup", 240, 190, true);
    expect(onUpdateKeypoints.mock.lastCall?.[1].keypoints).toEqual([{ x: 20, y: 20, v: 2 }, { x: 40, y: 40, v: 1 }]);
    expect(onKeypointPlacementChange).toHaveBeenLastCalledWith(null);

    // Dragging while a point is pending draws the next box instead of placing it.
    await act(async () => renderStage(1));
    onUpdateKeypoints.mockClear();
    await pointer("pointerdown", 210, 160);
    await pointer("pointermove", 260, 220);
    await pointer("pointerup", 260, 220);
    expect(onUpdateKeypoints).not.toHaveBeenCalled();
    expect(onKeypointPlacementChange).toHaveBeenLastCalledWith(null);
    expect(onCreateBox).toHaveBeenCalledWith({ x: 10, y: 10, width: 50, height: 60 }, "person");
  });

  it.each(["rectangle", "polygon"] as const)("asks for a class when the %s tool is used without one", async (tool) => {
    const onMissingClass = vi.fn();
    await act(async () => {
      root.render(<CanvasStage ref={stage}
        image={{ id: "image", fileName: "image.jpg", relativePath: "image.jpg", width: 100, height: 100, orientation: 1, status: "active" }}
        imageUrl="blob:image" restoredViewport={null} restoredDraft={null} activeTool={tool}
        activeClass={null} appendPolygonToId={null} classes={[]} annotations={[]} selectedAnnotationId={null}
        onSelectAnnotation={vi.fn()} onCreateBox={vi.fn()} onUpdateBox={vi.fn()} onCreatePolygon={vi.fn()} onMissingClass={onMissingClass}
        onUpdatePolygon={vi.fn()} onDraftChange={vi.fn()} onViewportChange={vi.fn()} onPointerChange={vi.fn()} onDecodeError={vi.fn()}
      />);
    });
    await act(async () => finishDecode());
    const canvas = host.querySelector<HTMLCanvasElement>(".interaction-canvas")!;
    await act(() => { canvas.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 250, clientY: 200, button: 0, pointerId: 1 })); });
    expect(onMissingClass).toHaveBeenCalledOnce();
  });

  it("uses a visible native cursor while the image is decoding", async () => {
    const canvas = await mount();
    expect(canvas.style.cursor).toBe("crosshair");
  });

  it("shows grab cursors while panning and restores the drawing cursor afterward", async () => {
    const canvas = await mount();
    await act(async () => finishDecode());
    await act(() => { window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space", key: " " })); });
    expect(canvas.style.cursor).toBe("grab");
    await act(() => { window.dispatchEvent(new KeyboardEvent("keyup", { code: "Space", key: " " })); });
    expect(canvas.style.cursor).toBe("none");
  });
});
