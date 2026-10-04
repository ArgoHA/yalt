// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as backend from "../backend";
import type { AnnotationRecord, ImageRecord, ProjectSummary } from "../types";
import { ProjectWorkspace } from "./ProjectWorkspace";

const canvas = vi.hoisted(() => ({ removeLastDraftPoint: vi.fn(), cancelDraft: vi.fn() }));
const cache = vi.hoisted(() => ({ load: vi.fn(), clear: vi.fn() }));
vi.mock("./CanvasStage", async () => {
  const { forwardRef, useImperativeHandle } = await import("react");
  return { CanvasStage: forwardRef((_props, ref) => {
    useImperativeHandle(ref, () => canvas);
    return <div data-testid="canvas" />;
  }) };
});
vi.mock("../editor/imageCache", () => ({ ImageUrlCache: class {
  load() { return cache.load(); }
  prefetch() {}
  clear() { cache.clear(); }
} }));
vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof backend>();
  return Object.fromEntries(Object.keys(original).map((key) => [key, vi.fn()]));
});

const project: ProjectSummary = {
  id: "project", name: "Test", rootPath: "/test", taskType: "segmentation", classificationMode: null,
  createdAtMs: 0, updatedAtMs: 0, imageCount: 2, missingImageCount: 0, deletedImageCount: 0, lastImageId: "first",
};
const images: ImageRecord[] = ["first", "second"].map((id) => ({
  id, relativePath: `${id}.jpg`, fileName: `${id}.jpg`, width: 100, height: 100, orientation: 1, status: "active",
}));
const annotation: AnnotationRecord = {
  id: "polygon", imageId: "first", classId: "class", kind: "polygon", isVisible: true,
  createdAtMs: 0, updatedAtMs: 0, geometry: { polygons: [[{ x: 10, y: 10 }, { x: 80, y: 10 }, { x: 40, y: 80 }]] },
};

describe("workspace interactions", () => {
  let host: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    localStorage.clear();
    cache.load.mockResolvedValue("blob:image");
    vi.mocked(backend.listProjectImages).mockImplementation(async () => images.map((image) => ({ ...image })));
    vi.mocked(backend.rescanProject).mockResolvedValue(project);
    vi.mocked(backend.getProjectSummary).mockResolvedValue(project);
    vi.mocked(backend.getHistoryState).mockResolvedValue({ canUndo: false, canRedo: false });
    vi.mocked(backend.getDetectionState).mockResolvedValue({
      activeClassId: "class", classes: [{ id: "class", name: "Object", position: 0, shortcut: "1", color: "#ff0000" }],
    });
    vi.mocked(backend.listPolygonAnnotations).mockImplementation(async (_root, id) => id === "first" ? [annotation] : []);
    vi.mocked(backend.loadViewportState).mockResolvedValue(null);
    vi.mocked(backend.loadPolygonDraft).mockResolvedValue(null);
    vi.mocked(backend.saveWorkspaceState).mockResolvedValue();
    vi.mocked(backend.deletePolygonAnnotation).mockResolvedValue();
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(() => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });
  async function mount() {
    await act(async () => { root.render(<ProjectWorkspace project={project} onProjectChange={vi.fn()} onClose={vi.fn()} />); });
    expect(host.querySelector('[data-testid="canvas"]')).not.toBeNull();
  }
  async function press(target: EventTarget, key: string) {
    await act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })); });
  }

  it.each(["Backspace", "Delete"])("%s removes a draft vertex before an existing object", async (key) => {
    await mount();
    canvas.removeLastDraftPoint.mockReturnValue(true);
    await press(window, key);
    expect(canvas.removeLastDraftPoint).toHaveBeenCalledOnce();
    expect(backend.deletePolygonAnnotation).not.toHaveBeenCalled();
  });

  it("leaves arrow and delete keys to the focused class dropdown", async () => {
    await mount();
    await act(() => host.querySelector<HTMLButtonElement>(".annotation-list button")!.click());
    const dropdown = host.querySelector<HTMLSelectElement>(".box-inspector select")!;
    expect(dropdown).not.toBeNull();
    dropdown.focus();
    await press(dropdown, "ArrowRight");
    await press(dropdown, "Delete");
    expect(host.querySelector(".image-row.active")?.textContent).toContain("first.jpg");
    expect(backend.deletePolygonAnnotation).not.toHaveBeenCalled();
  });

  it("does not run editor shortcuts from buttons in shortcut settings", async () => {
    await mount();
    await act(() => host.querySelector<HTMLButtonElement>('[aria-label="Project options"]')!.click());
    const shortcutButton = [...host.querySelectorAll<HTMLButtonElement>(".project-menu button")].find((button) => button.textContent?.includes("Customize shortcuts"))!;
    await act(() => shortcutButton.click());
    const button = host.querySelector<HTMLButtonElement>('[role="dialog"] button')!;
    expect(button).not.toBeNull();
    button.focus();
    await press(button, "Delete");
    await press(button, "ArrowRight");
    expect(backend.deletePolygonAnnotation).not.toHaveBeenCalled();
    expect(host.querySelector(".image-row.active")?.textContent).toContain("first.jpg");
  });

  it("does not restore old annotations when deletion finishes after navigating", async () => {
    let finishDelete!: () => void;
    vi.mocked(backend.deletePolygonAnnotation).mockReturnValue(new Promise<void>((resolve) => { finishDelete = resolve; }));
    // Keep another object on the first image, which must not appear on the second.
    vi.mocked(backend.listPolygonAnnotations).mockImplementation(async (_root, id) => id === "first" ? [annotation, { ...annotation, id: "second-polygon" }] : []);
    await mount();
    await press(window, "Delete");
    await press(window, "ArrowRight");
    expect(host.querySelector(".image-row.active")?.textContent).toContain("second.jpg");
    await act(async () => finishDelete());
    expect(host.querySelectorAll(".annotation-list button")).toHaveLength(0);
  });

  it("discards cached image bytes when rescanning changed source files", async () => {
    await mount();
    cache.clear.mockClear();
    cache.load.mockClear();
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Rescan image folder"]')!.click());
    expect(cache.clear).toHaveBeenCalledOnce();
    expect(cache.load).toHaveBeenCalledOnce();
    expect(cache.clear.mock.invocationCallOrder[0]).toBeLessThan(cache.load.mock.invocationCallOrder[0]);
  });
});
