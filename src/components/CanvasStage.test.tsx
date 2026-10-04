// @vitest-environment happy-dom
import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
