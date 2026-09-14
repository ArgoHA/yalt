import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  appendPolygonIsland,
  createBboxAnnotation,
  createPolygonAnnotation,
  createDetectionClass,
  deleteBboxAnnotation,
  deleteDetectionClass,
  deleteProjectImage,
  deletePolygonAnnotation,
  exportCocoAnnotations,
  exportClassificationCsv,
  exportClassDirectories,
  exportYoloAnnotations,
  exportYoloPolygons,
  exportCocoPolygons,
  getDetectionState,
  getClassificationState,
  getHistoryState,
  getProjectSummary,
  importCocoAnnotations,
  importClassificationCsv,
  importClassDirectories,
  importDetectionLabels,
  importYoloAnnotations,
  importYoloPolygons,
  importCocoPolygons,
  listImageAnnotations,
  listProjectImages,
  listPolygonAnnotations,
  loadBboxDraft,
  loadViewportState,
  loadPolygonDraft,
  redoProjectOperation,
  renameDetectionClass,
  rescanProject,
  restoreProjectImage,
  saveBboxDraft,
  saveWorkspaceState,
  savePolygonDraft,
  setActiveDetectionClass,
  setImageClassification,
  undoProjectOperation,
  updateBboxAnnotation,
  updatePolygonAnnotation,
} from "../backend";
import { ImageUrlCache } from "../editor/imageCache";
import { geometryArea, geometryWarnings } from "../editor/polygons";
import { loadShortcuts, matchesShortcut, saveShortcuts, shortcutLabel, type ShortcutMap } from "../editor/shortcuts";
import type {
  AnnotationDraft,
  AnnotationRecord,
  BboxAnnotationRecord,
  BboxGeometry,
  ClassRecord,
  EditorTool,
  HistoryState,
  ImageRecord,
  ProjectSummary,
  PolygonAnnotationRecord,
  PolygonGeometry,
  ViewportSnapshot,
} from "../types";
import type { Point } from "../editor/viewport";
import { CanvasStage, type CanvasStageHandle } from "./CanvasStage";
import { VirtualImageList } from "./VirtualImageList";
import { ShortcutSettings } from "./ShortcutSettings";

const TASK_LABELS = {
  detection: "Object detection",
  segmentation: "Polygon segmentation",
  classification: "Image classification",
};

const IMAGE_DELETE_SEQUENCE_MS = 800;

type IconName = "back" | "refresh" | "undo" | "redo" | "select" | "rectangle" | "polygon"
  | "fit" | "minus" | "plus" | "trash" | "restore" | "database" | "image" | "import" | "export" | "eye"
  | "leftPanel" | "rightPanel" | "more" | "copy";

function ToolIcon({ name }: { name: IconName }) {
  const paths: Record<IconName, React.ReactNode> = {
    back: <path d="m15 5-7 7 7 7" />,
    refresh: <path d="M19 8a8 8 0 1 0 1 7M19 4v4h-4" />,
    undo: <path d="M9 7 4 12l5 5M5 12h8a6 6 0 0 1 6 6" />,
    redo: <path d="m15 7 5 5-5 5M19 12h-8a6 6 0 0 0-6 6" />,
    select: <path d="m6 3 11 9-5 1 3 6-2.5 1.2-3-6L6 18z" />,
    rectangle: <rect x="4" y="5" width="16" height="14" rx="1" />,
    polygon: <path d="m5 17 2-11 10-2 3 10-7 6z" />,
    fit: <path d="M9 4H4v5M15 4h5v5M20 15v5h-5M9 20H4v-5" />,
    minus: <path d="M5 12h14" />,
    plus: <path d="M12 5v14M5 12h14" />,
    trash: <><path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13" /><path d="M10 11v5M14 11v5" /></>,
    restore: <><path d="M5 8v5h5" /><path d="M6.5 12a7 7 0 1 0 2-5" /></>,
    database: <><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7" /></>,
    image: <><rect x="3" y="4" width="18" height="16" rx="2" /><circle cx="8.5" cy="9" r="1.5" /><path d="m4 17 5-5 4 4 2-2 5 4" /></>,
    import: <><path d="M12 3v12M7 10l5 5 5-5" /><path d="M4 19h16" /></>,
    export: <><path d="M12 16V4M7 9l5-5 5 5" /><path d="M4 20h16" /></>,
    eye: <><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6" /><circle cx="12" cy="12" r="2.5" /></>,
    leftPanel: <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></>,
    rightPanel: <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M15 4v16" /></>,
    more: <><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></>,
    copy: <><rect x="8" y="8" width="11" height="11" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></>,
  };
  return <svg viewBox="0 0 24 24" aria-hidden="true">{paths[name]}</svg>;
}

function message(error: unknown): string {
  if (typeof error === "string") return error;
  return error instanceof Error ? error.message : "The operation could not be completed.";
}

function initialTool(project: ProjectSummary): EditorTool {
  if (project.taskType === "detection") return "rectangle";
  if (project.taskType === "segmentation") return "polygon";
  return "select";
}

export function ProjectWorkspace({
  project,
  onProjectChange,
  onClose,
}: {
  project: ProjectSummary;
  onProjectChange: (project: ProjectSummary) => void;
  onClose: () => void;
}) {
  const [currentProject, setCurrentProject] = useState(project);
  const [images, setImages] = useState<ImageRecord[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(project.lastImageId);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [loadedImageId, setLoadedImageId] = useState<string | null>(null);
  const [restoredViewport, setRestoredViewport] = useState<ViewportSnapshot | null | undefined>(undefined);
  const [history, setHistory] = useState<HistoryState>({ canUndo: false, canRedo: false });
  const [activeTool, setActiveTool] = useState<EditorTool>(initialTool(project));
  const [classes, setClasses] = useState<ClassRecord[]>([]);
  const [activeClassId, setActiveClassId] = useState<string | null>(null);
  const [imageClassIds, setImageClassIds] = useState<string[]>([]);
  const [labeledImageCount, setLabeledImageCount] = useState(0);
  const [annotations, setAnnotations] = useState<AnnotationRecord[]>([]);
  const [annotationsLoadedId, setAnnotationsLoadedId] = useState<string | null>(null);
  const [selectedAnnotationId, setSelectedAnnotationId] = useState<string | null>(null);
  const [appendPolygonToId, setAppendPolygonToId] = useState<string | null>(null);
  const [restoredDraft, setRestoredDraft] = useState<AnnotationDraft | null | undefined>(undefined);
  const [newClassName, setNewClassName] = useState("");
  const [editingClassId, setEditingClassId] = useState<string | null>(null);
  const [editingClassName, setEditingClassName] = useState("");
  const [showAddClass, setShowAddClass] = useState(false);
  const [showProjectMenu, setShowProjectMenu] = useState(false);
  const [showShortcutSettings, setShowShortcutSettings] = useState(false);
  const [shortcuts, setShortcuts] = useState<ShortcutMap>(() => loadShortcuts());
  const [leftPanelVisible, setLeftPanelVisible] = useState(() => panelPreference("yalt.images-panel", true, "labeler.images-panel"));
  const [rightPanelVisible, setRightPanelVisible] = useState(() => panelPreference("yalt.objects-panel", true, "labeler.objects-panel"));
  const [zoomPercent, setZoomPercent] = useState(100);
  const [pointer, setPointer] = useState<Point | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ text: string; undoable: boolean } | null>(null);
  const cacheRef = useRef(new ImageUrlCache(3));
  const canvasRef = useRef<CanvasStageHandle>(null);
  const viewportSaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingViewportSave = useRef<{ imageId: string; snapshot: ViewportSnapshot } | null>(null);
  const workspaceSaveQueue = useRef<Promise<void>>(Promise.resolve());
  const draftSaveQueue = useRef<Promise<void>>(Promise.resolve());
  const selectedIdRef = useRef(selectedId);
  const activeClassSaveQueue = useRef<Promise<void>>(Promise.resolve());
  const initialLastImageId = useRef(project.lastImageId);
  const pendingImageDeleteAtRef = useRef<number | null>(null);

  const activeImages = useMemo(() => images.filter((image) => image.status === "active"), [images]);
  const selected = useMemo(() => images.find((image) => image.id === selectedId) ?? null, [images, selectedId]);
  const activePosition = selected ? activeImages.findIndex((image) => image.id === selected.id) : -1;
  const activeClass = useMemo(() => classes.find((item) => item.id === activeClassId) ?? null, [activeClassId, classes]);
  const selectedAnnotation = useMemo(() => annotations.find((item) => item.id === selectedAnnotationId) ?? null, [annotations, selectedAnnotationId]);

  useEffect(() => { selectedIdRef.current = selectedId; }, [selectedId]);
  useEffect(() => { pendingImageDeleteAtRef.current = null; }, [selectedId]);
  useEffect(() => { localStorage.setItem("yalt.images-panel", leftPanelVisible ? "visible" : "hidden"); }, [leftPanelVisible]);
  useEffect(() => { localStorage.setItem("yalt.objects-panel", rightPanelVisible ? "visible" : "hidden"); }, [rightPanelVisible]);

  const commitProject = useCallback((next: ProjectSummary) => {
    setCurrentProject(next);
    onProjectChange(next);
  }, [onProjectChange]);

  const queueWorkspaceSave = useCallback((imageId: string, viewport: ViewportSnapshot | null) => {
    const next = workspaceSaveQueue.current
      .catch(() => undefined)
      .then(() => saveWorkspaceState(project.rootPath, imageId, viewport));
    workspaceSaveQueue.current = next;
    return next;
  }, [project.rootPath]);

  const flushViewportSave = useCallback(() => {
    if (viewportSaveTimer.current) clearTimeout(viewportSaveTimer.current);
    viewportSaveTimer.current = null;
    const pending = pendingViewportSave.current;
    pendingViewportSave.current = null;
    if (!pending) return;
    void queueWorkspaceSave(pending.imageId, pending.snapshot)
      .catch((reason) => {
        setError(message(reason));
      });
  }, [queueWorkspaceSave]);

  const reloadMetadata = useCallback(async (preferredId?: string | null) => {
    const [loadedImages, summary, historyState] = await Promise.all([
      listProjectImages(project.rootPath),
      getProjectSummary(project.rootPath),
      getHistoryState(project.rootPath),
    ]);
    setImages(loadedImages);
    setHistory(historyState);
    commitProject(summary);
    setSelectedId((current) => {
      const candidate = preferredId ?? current ?? summary.lastImageId;
      if (candidate && loadedImages.some((image) => image.id === candidate)) return candidate;
      return loadedImages.find((image) => image.status === "active")?.id ?? loadedImages[0]?.id ?? null;
    });
  }, [commitProject, project.rootPath]);

  useEffect(() => {
    reloadMetadata(initialLastImageId.current).catch((reason) => setError(message(reason)));
  }, [reloadMetadata]);

  const reloadSpatialState = useCallback(async () => {
    if (project.taskType === "classification") {
      const state = await getClassificationState(project.rootPath, null);
      setClasses(state.classes);
      setLabeledImageCount(state.labeledImageCount);
      setActiveClassId((current) => current && state.classes.some((item) => item.id === current) ? current : state.classes[0]?.id ?? null);
    } else {
      const state = await getDetectionState(project.rootPath);
      setClasses(state.classes);
      setActiveClassId(state.activeClassId);
    }
  }, [project.rootPath, project.taskType]);

  useEffect(() => {
    reloadSpatialState().catch((reason) => setError(message(reason)));
  }, [reloadSpatialState]);

  useEffect(() => () => {
    flushViewportSave();
    cacheRef.current.clear();
  }, [flushViewportSave]);

  useEffect(() => {
    flushViewportSave();
    setImageUrl(null);
    setLoadedImageId(null);
    setRestoredViewport(undefined);
    setRestoredDraft(undefined);
    setAnnotations([]);
    setAnnotationsLoadedId(null);
    setSelectedAnnotationId(null);
    setImageClassIds([]);
    setPointer(null);
    if (!selected || selected.status !== "active") {
      setRestoredViewport(null);
      setRestoredDraft(null);
      return;
    }

    let cancelled = false;
    Promise.all([
      cacheRef.current.load(project.rootPath, selected),
      loadViewportState(project.rootPath, selected.id),
      project.taskType === "detection" ? listImageAnnotations(project.rootPath, selected.id)
        : project.taskType === "segmentation" ? listPolygonAnnotations(project.rootPath, selected.id) : Promise.resolve([]),
      project.taskType === "detection" ? loadBboxDraft(project.rootPath, selected.id)
        : project.taskType === "segmentation" ? loadPolygonDraft(project.rootPath, selected.id) : Promise.resolve(null),
      project.taskType === "classification" ? getClassificationState(project.rootPath, selected.id) : Promise.resolve(null),
    ]).then(([url, viewport, loadedAnnotations, draft, classification]) => {
      if (cancelled) return;
      setImageUrl(url);
      setLoadedImageId(selected.id);
      setRestoredViewport(viewport);
      setAnnotations(loadedAnnotations);
      setAnnotationsLoadedId(selected.id);
      setRestoredDraft(draft);
      if (classification) {
        setClasses(classification.classes);
        setImageClassIds(classification.imageClassIds);
        setLabeledImageCount(classification.labeledImageCount);
      }
      void queueWorkspaceSave(selected.id, null).catch((reason) => setError(message(reason)));
      const index = activeImages.findIndex((image) => image.id === selected.id);
      cacheRef.current.prefetch(
        project.rootPath,
        [activeImages[index - 1], activeImages[index + 1]].filter(Boolean),
      );
    }).catch((reason) => !cancelled && setError(message(reason)));
    return () => { cancelled = true; };
  }, [activeImages, flushViewportSave, project.rootPath, project.taskType, queueWorkspaceSave, selected]);

  const selectImage = useCallback((image: ImageRecord) => {
    setNotice(null);
    setAppendPolygonToId(null);
    setSelectedId(image.id);
  }, []);

  const navigate = useCallback((delta: number) => {
    if (activeImages.length === 0) return;
    const current = activeImages.findIndex((image) => image.id === selectedId);
    const next = current < 0
      ? activeImages[0]
      : activeImages[Math.min(activeImages.length - 1, Math.max(0, current + delta))];
    if (next) selectImage(next);
  }, [activeImages, selectImage, selectedId]);

  const refreshAfterHistory = useCallback(async (preferredId?: string | null) => {
    await reloadMetadata(preferredId);
    const imageId = preferredId ?? selectedId;
    if ((project.taskType === "detection" || project.taskType === "segmentation") && imageId) {
      setAnnotations(project.taskType === "detection"
        ? await listImageAnnotations(project.rootPath, imageId)
        : await listPolygonAnnotations(project.rootPath, imageId));
      setAnnotationsLoadedId(imageId);
      setSelectedAnnotationId(null);
    } else if (project.taskType === "classification" && imageId) {
      const state = await getClassificationState(project.rootPath, imageId);
      setClasses(state.classes);
      setImageClassIds(state.imageClassIds);
      setLabeledImageCount(state.labeledImageCount);
    }
    setNotice(null);
  }, [project.rootPath, project.taskType, reloadMetadata, selectedId]);

  const undo = useCallback(async () => {
    if (busy || !history.canUndo) return;
    setBusy(true);
    try {
      const affectedImageId = await undoProjectOperation(project.rootPath);
      if (affectedImageId) await refreshAfterHistory(affectedImageId);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, history.canUndo, project.rootPath, refreshAfterHistory]);

  const redo = useCallback(async () => {
    if (busy || !history.canRedo) return;
    setBusy(true);
    try {
      const affectedImageId = await redoProjectOperation(project.rootPath);
      if (affectedImageId) await refreshAfterHistory(affectedImageId);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, history.canRedo, project.rootPath, refreshAfterHistory]);

  const moveSelectedToDeleted = useCallback(async () => {
    if (busy || !selected || selected.status !== "active") return;
    if (project.taskType !== "classification" && annotationsLoadedId !== selected.id) {
      setNotice({ text: "Wait for this image's annotations to finish loading.", undoable: false });
      return;
    }
    if (project.taskType !== "classification" && annotations.length > 0) {
      setNotice({
        text: `Delete all ${annotations.length} object${annotations.length === 1 ? "" : "s"} first. Backspace removes them one at a time.`,
        undoable: false,
      });
      return;
    }
    const index = activeImages.findIndex((image) => image.id === selected.id);
    const nextId = activeImages[index + 1]?.id ?? activeImages[index - 1]?.id ?? null;
    setBusy(true);
    try {
      const summary = await deleteProjectImage(project.rootPath, selected.id);
      commitProject(summary);
      await reloadMetadata(nextId);
      setNotice({ text: `${selected.fileName} moved to deleted.`, undoable: true });
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [activeImages, annotations.length, annotationsLoadedId, busy, commitProject, project.rootPath, project.taskType, reloadMetadata, selected]);

  const restoreSelected = useCallback(async () => {
    if (busy || !selected || selected.status !== "deleted") return;
    setBusy(true);
    try {
      const summary = await restoreProjectImage(project.rootPath, selected.id);
      commitProject(summary);
      await reloadMetadata(selected.id);
      setNotice({ text: `${selected.fileName} restored.`, undoable: true });
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, commitProject, project.rootPath, reloadMetadata, selected]);

  const rescan = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const summary = await rescanProject(project.rootPath);
      commitProject(summary);
      await reloadMetadata();
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, commitProject, project.rootPath, reloadMetadata]);

  const refreshHistoryState = useCallback(async () => {
    setHistory(await getHistoryState(project.rootPath));
  }, [project.rootPath]);

  const chooseClass = useCallback((classId: string, enterDrawMode = true) => {
    setAppendPolygonToId(null);
    setActiveClassId(classId);
    if (enterDrawMode && project.taskType === "detection") setActiveTool("rectangle");
    if (enterDrawMode && project.taskType === "segmentation") setActiveTool("polygon");
    const next = activeClassSaveQueue.current.catch(() => undefined).then(() => setActiveDetectionClass(project.rootPath, classId));
    activeClassSaveQueue.current = next;
    void next.catch((reason) => setError(message(reason)));
  }, [project.rootPath, project.taskType]);

  const addClass = useCallback(async () => {
    if (!newClassName.trim() || busy) return;
    setBusy(true);
    try {
      const created = await createDetectionClass(project.rootPath, newClassName);
      setClasses((current) => [...current, created]);
      setNewClassName("");
      setShowAddClass(false);
      chooseClass(created.id);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, chooseClass, newClassName, project.rootPath]);

  const commitClassRename = useCallback(async () => {
    const classId = editingClassId;
    const name = editingClassName.trim();
    setEditingClassId(null);
    if (!classId || !name) return;
    try {
      const updated = await renameDetectionClass(project.rootPath, classId, name);
      setClasses((current) => current.map((item) => item.id === classId ? updated : item));
    } catch (reason) {
      setError(message(reason));
    }
  }, [editingClassId, editingClassName, project.rootPath]);

  const removeClass = useCallback(async (classId: string) => {
    if (busy) return;
    setBusy(true);
    try {
      const state = await deleteDetectionClass(project.rootPath, classId);
      setClasses(state.classes);
      setActiveClassId(state.activeClassId);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, project.rootPath]);

  const classifyImage = useCallback(async (classId: string) => {
    if (busy || !selected || selected.status !== "active" || project.taskType !== "classification") return;
    const single = project.classificationMode !== "multi";
    const nextClassIds = single
      ? [classId]
      : imageClassIds.includes(classId) ? imageClassIds.filter((id) => id !== classId) : [...imageClassIds, classId];
    const index = activeImages.findIndex((image) => image.id === selected.id);
    const nextId = single ? activeImages[index + 1]?.id ?? selected.id : selected.id;
    setBusy(true);
    setError(null);
    try {
      const state = await setImageClassification(project.rootPath, selected.id, nextClassIds);
      setClasses(state.classes);
      setImageClassIds(state.imageClassIds);
      setLabeledImageCount(state.labeledImageCount);
      setActiveClassId(classId);
      await reloadMetadata(nextId);
      setNotice({
        text: single ? `${selected.fileName} classified; advanced to the next image.` : `${selected.fileName} labels updated.`,
        undoable: true,
      });
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [activeImages, busy, imageClassIds, project.classificationMode, project.rootPath, project.taskType, reloadMetadata, selected]);

  const clearImageClassification = useCallback(async () => {
    if (busy || !selected || selected.status !== "active") return;
    setBusy(true);
    try {
      const state = await setImageClassification(project.rootPath, selected.id, []);
      setImageClassIds(state.imageClassIds);
      setLabeledImageCount(state.labeledImageCount);
      await reloadMetadata(selected.id);
      setNotice({ text: `${selected.fileName} is now unclassified.`, undoable: true });
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, project.rootPath, reloadMetadata, selected]);

  const copySelectedPath = useCallback(async () => {
    if (!selected) return;
    const root = project.rootPath.replace(/[\\/]+$/, "");
    const absolutePath = `${root}/${selected.relativePath}`;
    try {
      await navigator.clipboard.writeText(absolutePath);
      setNotice({ text: "Image path copied.", undoable: false });
    } catch (reason) {
      setError(`Could not copy the image path: ${message(reason)}`);
    }
  }, [project.rootPath, selected]);

  const createBox = useCallback(async (geometry: BboxGeometry, classId: string) => {
    if (!selected) return;
    const imageId = selected.id;
    try {
      await draftSaveQueue.current.catch(() => undefined);
      const created = await createBboxAnnotation(project.rootPath, imageId, classId, geometry);
      if (selectedIdRef.current === imageId) {
        setAnnotations((current) => [...current, created]);
        setSelectedAnnotationId(created.id);
      }
      await refreshHistoryState();
    } catch (reason) {
      setError(message(reason));
    }
  }, [project.rootPath, refreshHistoryState, selected]);

  const updateBoxGeometry = useCallback(async (annotation: BboxAnnotationRecord, geometry: BboxGeometry) => {
    try {
      const updated = await updateBboxAnnotation(project.rootPath, annotation.id, annotation.classId, geometry, annotation.isVisible);
      setAnnotations((current) => current.map((item) => item.id === updated.id ? updated : item));
      await refreshHistoryState();
    } catch (reason) {
      setError(message(reason));
      if (selected) setAnnotations(await listImageAnnotations(project.rootPath, selected.id));
    }
  }, [project.rootPath, refreshHistoryState, selected]);

  const createPolygon = useCallback(async (geometry: PolygonGeometry, classId: string, annotationId?: string | null) => {
    if (!selected) return;
    const imageId = selected.id;
    setAppendPolygonToId(null);
    try {
      await draftSaveQueue.current.catch(() => undefined);
      const created = annotationId
        ? await appendPolygonIsland(project.rootPath, annotationId, geometry.polygons[0])
        : await createPolygonAnnotation(project.rootPath, imageId, classId, geometry);
      if (selectedIdRef.current === imageId) {
        setAnnotations((current) => annotationId
          ? current.map((item) => item.id === created.id ? created : item)
          : [...current, created]);
        setSelectedAnnotationId(created.id);
      }
      await refreshHistoryState();
    } catch (reason) {
      setError(message(reason));
    }
  }, [project.rootPath, refreshHistoryState, selected]);

  const beginIsland = useCallback(() => {
    if (!selectedAnnotation || selectedAnnotation.kind !== "polygon") return;
    canvasRef.current?.cancelDraft();
    chooseClass(selectedAnnotation.classId, false);
    setAppendPolygonToId(selectedAnnotation.id);
    setActiveTool("polygon");
  }, [chooseClass, selectedAnnotation]);

  const updatePolygonGeometry = useCallback(async (annotation: PolygonAnnotationRecord, geometry: PolygonGeometry) => {
    try {
      const updated = await updatePolygonAnnotation(project.rootPath, annotation.id, annotation.classId, geometry, annotation.isVisible);
      setAnnotations((current) => current.map((item) => item.id === updated.id ? updated : item));
      await refreshHistoryState();
    } catch (reason) {
      setError(message(reason));
      if (selected) setAnnotations(await listPolygonAnnotations(project.rootPath, selected.id));
    }
  }, [project.rootPath, refreshHistoryState, selected]);

  const changeSelectedAnnotation = useCallback(async (changes: { classId?: string; isVisible?: boolean }) => {
    if (!selectedAnnotation) return;
    try {
      const classId = changes.classId ?? selectedAnnotation.classId;
      const isVisible = changes.isVisible ?? selectedAnnotation.isVisible;
      const updated = selectedAnnotation.kind === "bbox"
        ? await updateBboxAnnotation(project.rootPath, selectedAnnotation.id, classId, selectedAnnotation.geometry, isVisible)
        : await updatePolygonAnnotation(project.rootPath, selectedAnnotation.id, classId, selectedAnnotation.geometry, isVisible);
      setAnnotations((current) => current.map((item) => item.id === updated.id ? updated : item));
      if (changes.classId) chooseClass(changes.classId, false);
      await refreshHistoryState();
    } catch (reason) {
      setError(message(reason));
    }
  }, [chooseClass, project.rootPath, refreshHistoryState, selectedAnnotation]);

  const removeAnnotation = useCallback(async (annotation: AnnotationRecord | null = selectedAnnotation) => {
    if (!annotation) return;
    try {
      if (annotation.kind === "bbox") await deleteBboxAnnotation(project.rootPath, annotation.id);
      else await deletePolygonAnnotation(project.rootPath, annotation.id);
      const remaining = annotations.filter((item) => item.id !== annotation.id);
      setAnnotations(remaining);
      setSelectedAnnotationId(remaining.at(-1)?.id ?? null);
      await refreshHistoryState();
    } catch (reason) {
      setError(message(reason));
    }
  }, [annotations, project.rootPath, refreshHistoryState, selectedAnnotation]);

  const changeDraft = useCallback((draft: AnnotationDraft | null) => {
    if (!selected) return;
    if (!draft) setAppendPolygonToId(null);
    setRestoredDraft(draft);
    const next = draftSaveQueue.current.catch(() => undefined).then(() => project.taskType === "segmentation"
      ? savePolygonDraft(project.rootPath, selected.id, draft && "points" in draft ? draft : null)
      : saveBboxDraft(project.rootPath, selected.id, draft && "x" in draft ? draft : null));
    draftSaveQueue.current = next;
    void next.catch((reason) => setError(message(reason)));
  }, [project.rootPath, project.taskType, selected]);

  const runExchange = useCallback(async (kind: "labels" | "yolo-import" | "yolo-export" | "yolo-polygon-import" | "yolo-polygon-export" | "coco-import" | "coco-export" | "coco-polygon-import" | "coco-polygon-export" | "classification-csv-import" | "classification-csv-export" | "class-directories-import" | "class-directories-export") => {
    if (busy) return;
    setShowProjectMenu(false);
    let selectedPath: string | null = null;
    if (kind === "classification-csv-import") {
      const result = await openDialog({ multiple: false, filters: [{ name: "Classification CSV", extensions: ["csv"] }], title: "Import image classifications" });
      selectedPath = typeof result === "string" ? result : null;
    } else if (kind === "classification-csv-export") {
      selectedPath = await saveDialog({ defaultPath: "classifications.csv", filters: [{ name: "CSV", extensions: ["csv"] }], title: "Export image classifications" });
    } else if (kind === "class-directories-import") {
      const result = await openDialog({ directory: true, multiple: false, title: "Choose folder containing class directories" });
      selectedPath = typeof result === "string" ? result : null;
    } else if (kind === "class-directories-export") {
      selectedPath = await saveDialog({ defaultPath: "classified-images", title: "Create class-directory export" });
    } else if (kind === "labels") {
      const result = await openDialog({ multiple: false, filters: [{ name: "Labels", extensions: ["txt"] }], title: "Load labels.txt" });
      selectedPath = typeof result === "string" ? result : null;
    } else if (kind === "yolo-import" || kind === "yolo-polygon-import") {
      const result = await openDialog({ directory: true, multiple: false, title: "Choose YOLO labels folder" });
      selectedPath = typeof result === "string" ? result : null;
    } else if (kind === "yolo-export" || kind === "yolo-polygon-export") {
      selectedPath = await saveDialog({ defaultPath: "labels-yolo", title: "Create YOLO export folder" });
    } else if (kind === "coco-import" || kind === "coco-polygon-import") {
      const result = await openDialog({ multiple: false, filters: [{ name: "COCO JSON", extensions: ["json"] }], title: "Import COCO annotations" });
      selectedPath = typeof result === "string" ? result : null;
    } else {
      selectedPath = await saveDialog({ defaultPath: "annotations.json", filters: [{ name: "COCO JSON", extensions: ["json"] }], title: "Export COCO annotations" });
    }
    if (!selectedPath) return;
    setBusy(true);
    try {
      if (kind.startsWith("classification-") || kind.startsWith("class-directories-")) {
        const report = kind === "classification-csv-import" ? await importClassificationCsv(project.rootPath, selectedPath)
          : kind === "classification-csv-export" ? await exportClassificationCsv(project.rootPath, selectedPath)
            : kind === "class-directories-import" ? await importClassDirectories(project.rootPath, selectedPath)
              : await exportClassDirectories(project.rootPath, selectedPath);
        setNotice({ text: report.message, undoable: kind.endsWith("import") });
        if (kind.endsWith("import")) {
          await reloadMetadata(selected?.id);
          await reloadSpatialState();
          if (selected) {
            const state = await getClassificationState(project.rootPath, selected.id);
            setImageClassIds(state.imageClassIds);
            setLabeledImageCount(state.labeledImageCount);
          }
        }
      } else if (kind === "labels") {
        const state = await importDetectionLabels(project.rootPath, selectedPath);
        setClasses(state.classes); setActiveClassId(state.activeClassId); setNotice({ text: `Loaded ${state.classes.length} classes.`, undoable: false });
      } else {
        const report = kind === "yolo-import" ? await importYoloAnnotations(project.rootPath, selectedPath)
          : kind === "yolo-export" ? await exportYoloAnnotations(project.rootPath, selectedPath)
            : kind === "yolo-polygon-import" ? await importYoloPolygons(project.rootPath, selectedPath)
              : kind === "yolo-polygon-export" ? await exportYoloPolygons(project.rootPath, selectedPath)
                : kind === "coco-import" ? await importCocoAnnotations(project.rootPath, selectedPath)
                  : kind === "coco-export" ? await exportCocoAnnotations(project.rootPath, selectedPath)
                    : kind === "coco-polygon-import" ? await importCocoPolygons(project.rootPath, selectedPath)
                      : await exportCocoPolygons(project.rootPath, selectedPath);
        setNotice({ text: report.message, undoable: kind.endsWith("import") });
        if (kind.endsWith("import")) {
          await reloadSpatialState();
          if (selected) setAnnotations(project.taskType === "segmentation"
            ? await listPolygonAnnotations(project.rootPath, selected.id)
            : await listImageAnnotations(project.rootPath, selected.id));
          await refreshHistoryState();
        }
      }
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }, [busy, project.rootPath, project.taskType, refreshHistoryState, reloadSpatialState, selected]);

  const allowedTools = useMemo<EditorTool[]>(() => {
    if (project.taskType === "detection") return ["select", "rectangle"];
    if (project.taskType === "segmentation") return ["select", "polygon"];
    return ["select"];
  }, [project.taskType]);

  const activateTool = useCallback((tool: EditorTool) => {
    if (tool !== activeTool || appendPolygonToId) canvasRef.current?.cancelDraft();
    setAppendPolygonToId(null);
    setActiveTool(tool);
  }, [activeTool, appendPolygonToId]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (isTypingTarget(event.target) || event.code === "Space") {
        pendingImageDeleteAtRef.current = null;
        return;
      }
      const command = event.metaKey || event.ctrlKey;
      const imageDeleteKey = !command && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "d";
      if (!imageDeleteKey) pendingImageDeleteAtRef.current = null;
      if (command && event.key.toLowerCase() === "z") {
        event.preventDefault();
        void (event.shiftKey ? redo() : undo());
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        navigate(-1);
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        navigate(1);
      } else if (matchesShortcut(event, shortcuts.fit)) {
        event.preventDefault();
        canvasRef.current?.fit();
      } else if (command && (event.key === "+" || event.key === "=")) {
        event.preventDefault();
        canvasRef.current?.zoomIn();
      } else if (command && event.key === "-") {
        event.preventDefault();
        canvasRef.current?.zoomOut();
      } else if (event.key === "Enter" && canvasRef.current?.finishDraft()) {
        event.preventDefault();
      } else if (event.key === "Backspace" || event.key === "Delete") {
        event.preventDefault();
        if (event.key === "Backspace" && canvasRef.current?.removeLastDraftPoint()) return;
        if (event.repeat) return;
        void removeAnnotation(selectedAnnotation ?? annotations.at(-1) ?? null);
      } else if (event.key === "Escape") {
        canvasRef.current?.cancelDraft();
        setAppendPolygonToId(null);
        setSelectedAnnotationId(null);
        setShowProjectMenu(false);
        setShowAddClass(false);
      } else if (imageDeleteKey) {
        event.preventDefault();
        if (event.repeat || !selected || selected.status !== "active") return;
        if (project.taskType !== "classification" && annotationsLoadedId !== selected.id) {
          pendingImageDeleteAtRef.current = null;
          setNotice({ text: "Wait for this image's annotations to finish loading.", undoable: false });
          return;
        }
        if (project.taskType !== "classification" && annotations.length > 0) {
          pendingImageDeleteAtRef.current = null;
          setNotice({
            text: `Delete all ${annotations.length} object${annotations.length === 1 ? "" : "s"} first. Backspace removes them one at a time.`,
            undoable: false,
          });
          return;
        }
        const now = performance.now();
        const previous = pendingImageDeleteAtRef.current;
        if (previous !== null && now - previous <= IMAGE_DELETE_SEQUENCE_MS) {
          pendingImageDeleteAtRef.current = null;
          void moveSelectedToDeleted();
        } else {
          pendingImageDeleteAtRef.current = now;
          setNotice({ text: "Press D again to move this image to deleted.", undoable: false });
        }
      } else if (!command && /^[0-9]$/.test(event.key)) {
        const classRecord = classes.find((item) => item.shortcut === event.key);
        if (classRecord) {
          event.preventDefault();
          if (project.taskType === "classification") void classifyImage(classRecord.id);
          else chooseClass(classRecord.id);
        }
      } else if (matchesShortcut(event, shortcuts.previousClass) || matchesShortcut(event, shortcuts.nextClass)) {
        const current = classes.findIndex((item) => item.id === activeClassId);
        const delta = matchesShortcut(event, shortcuts.previousClass) ? -1 : 1;
        const next = classes[Math.min(classes.length - 1, Math.max(0, (current < 0 ? 0 : current) + delta))];
        if (next) { event.preventDefault(); chooseClass(next.id); }
      } else if (matchesShortcut(event, shortcuts.island) && selectedAnnotation?.kind === "polygon") {
        event.preventDefault();
        beginIsland();
      } else if (matchesShortcut(event, shortcuts.edit)) {
        canvasRef.current?.cancelDraft();
        setAppendPolygonToId(null);
        setActiveTool("select");
      } else if (matchesShortcut(event, shortcuts.rectangle) && allowedTools.includes("rectangle")) {
        activateTool("rectangle");
      } else if (matchesShortcut(event, shortcuts.polygon) && allowedTools.includes("polygon")) {
        activateTool("polygon");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeClassId, activateTool, allowedTools, annotations, annotationsLoadedId, beginIsland, chooseClass, classes, classifyImage, moveSelectedToDeleted, navigate, project.taskType, redo, removeAnnotation, selected, selectedAnnotation, shortcuts, undo]);

  const commitShortcuts = useCallback((next: ShortcutMap) => {
    saveShortcuts(next);
    setShortcuts(next);
    setShowShortcutSettings(false);
    setNotice({ text: "Keyboard shortcuts saved on this Mac.", undoable: false });
  }, []);

  const scheduleViewportSave = useCallback((
    snapshot: ViewportSnapshot,
    percent: number,
    persist: boolean,
  ) => {
    setZoomPercent((current) => current === percent ? current : percent);
    if (!persist) return;
    if (!selected || selected.status !== "active") return;
    if (viewportSaveTimer.current) clearTimeout(viewportSaveTimer.current);
    pendingViewportSave.current = { imageId: selected.id, snapshot };
    viewportSaveTimer.current = setTimeout(flushViewportSave, 700);
  }, [flushViewportSave, selected]);

  return (
    <main className={`workspace-shell phase-two phase-three${project.taskType === "classification" ? " phase-five" : ""}`}>
      <header className="workspace-toolbar">
        <button className="back-button" onClick={onClose}><ToolIcon name="back" />Projects</button>
        <div className="project-identity"><strong>{currentProject.name}</strong><span>{TASK_LABELS[currentProject.taskType]}</span></div>
        <div className="workspace-path" title={currentProject.rootPath}>{currentProject.rootPath}</div>
        <span className="saved-state"><span />Saved locally</span>
        <div className="history-buttons">
          <button className="icon-button" aria-label="Undo" title="Undo (⌘Z)" disabled={!history.canUndo || busy} onClick={() => void undo()}><ToolIcon name="undo" /></button>
          <button className="icon-button" aria-label="Redo" title="Redo (⇧⌘Z)" disabled={!history.canRedo || busy} onClick={() => void redo()}><ToolIcon name="redo" /></button>
          <button className="icon-button" aria-label="Rescan image folder" title="Rescan image folder" disabled={busy} onClick={() => void rescan()}><ToolIcon name="refresh" /></button>
        </div>
      </header>

      {error && <div className="workspace-error" role="alert">{error}<button onClick={() => setError(null)}>Dismiss</button></div>}
      {notice && <div className="undo-notice" role="status"><span>{notice.text}</span>{notice.undoable && <button disabled={!history.canUndo || busy} onClick={() => void undo()}>Undo</button>}<button aria-label="Dismiss notification" onClick={() => setNotice(null)}>×</button></div>}

      <section className={`workspace-body${leftPanelVisible ? "" : " left-collapsed"}${rightPanelVisible ? "" : " right-collapsed"}`}>
        <aside className="image-rail">
          <div className="rail-heading"><strong>Images</strong><span>{currentProject.imageCount} available</span></div>
          <VirtualImageList images={images} selectedId={selectedId} onSelect={selectImage} />
          {images.length === 0 && <div className="empty-list">No JPEG, PNG, WebP, or TIFF images found.</div>}
          <div className="rail-foot">{currentProject.deletedImageCount} deleted · {currentProject.missingImageCount} missing</div>
        </aside>

        <section className="editor-column">
          <div className="tool-strip" aria-label="Editor tools">
            <div className="tool-group">
              <button className={leftPanelVisible ? "icon-button panel-toggle active" : "icon-button panel-toggle"} aria-label={`${leftPanelVisible ? "Hide" : "Show"} image list`} aria-pressed={leftPanelVisible} title={`${leftPanelVisible ? "Hide" : "Show"} image list`} onClick={() => setLeftPanelVisible((visible) => !visible)}><ToolIcon name="leftPanel" /></button>
              {allowedTools.map((tool) => (
                <button className={activeTool === tool ? "tool-button active" : "tool-button"} aria-pressed={activeTool === tool} onClick={() => activateTool(tool)} title={`${toolLabel(tool)} (${toolShortcut(tool, shortcuts)})`} key={tool}>
                  <ToolIcon name={tool} /><span>{toolLabel(tool)}</span><kbd>{toolShortcut(tool, shortcuts)}</kbd>
                </button>
              ))}
              {(project.taskType === "detection" || project.taskType === "segmentation") && activeClass && (
                <span className="active-class-chip" style={{ "--class-color": activeClass.color } as React.CSSProperties} title="Active class">
                  <i />{activeClass.name}<kbd>{activeClass.shortcut ?? "—"}</kbd>
                </span>
              )}
            </div>
            <div className="tool-group viewport-tools">
              <button className="icon-button" aria-label="Zoom out" title="Zoom out (⌘−)" onClick={() => canvasRef.current?.zoomOut()}><ToolIcon name="minus" /></button>
              <button className="zoom-readout" aria-label={`Zoom ${zoomPercent} percent; fit image`} onClick={() => canvasRef.current?.fit()} title="Fit image">{zoomPercent}%</button>
              <button className="icon-button" aria-label="Zoom in" title="Zoom in (⌘+)" onClick={() => canvasRef.current?.zoomIn()}><ToolIcon name="plus" /></button>
              <button className="icon-button" aria-label="Fit image" title={`Fit image (${shortcutLabel(shortcuts.fit)})`} onClick={() => canvasRef.current?.fit()}><ToolIcon name="fit" /></button>
            </div>
            <button className="delete-tool" disabled={!selected || selected.status !== "active" || busy} title="Move image to deleted (D D)" onClick={() => void moveSelectedToDeleted()}><ToolIcon name="trash" />Delete image</button>
            <button className={rightPanelVisible ? "icon-button panel-toggle active right-toggle" : "icon-button panel-toggle right-toggle"} aria-label={`${rightPanelVisible ? "Hide" : "Show"} ${project.taskType === "classification" ? "label" : "object"} panel`} aria-pressed={rightPanelVisible} title={`${rightPanelVisible ? "Hide" : "Show"} ${project.taskType === "classification" ? "label" : "object"} panel`} onClick={() => setRightPanelVisible((visible) => { if (visible) setShowProjectMenu(false); return !visible; })}><ToolIcon name="rightPanel" /></button>
          </div>

          <div className="stage-wrap">
            {selected?.status === "active" && loadedImageId === selected.id && annotationsLoadedId === selected.id && imageUrl && restoredViewport !== undefined && restoredDraft !== undefined ? (
              <CanvasStage
                ref={canvasRef}
                image={selected}
                imageUrl={imageUrl}
                restoredViewport={restoredViewport}
                restoredDraft={restoredDraft}
                activeTool={activeTool}
                activeClass={activeClass}
                appendPolygonToId={appendPolygonToId}
                classes={classes}
                annotations={annotations}
                selectedAnnotationId={selectedAnnotationId}
                onSelectAnnotation={setSelectedAnnotationId}
                onCreateBox={createBox}
                onUpdateBox={updateBoxGeometry}
                onCreatePolygon={createPolygon}
                onUpdatePolygon={updatePolygonGeometry}
                onDraftChange={changeDraft}
                onViewportChange={scheduleViewportSave}
                onPointerChange={setPointer}
                onDecodeError={setError}
              />
            ) : selected?.status === "deleted" ? (
              <div className="unavailable-stage deleted-stage"><ToolIcon name="trash" /><strong>{selected.fileName} is in deleted</strong><span>It is outside the active dataset and can be restored.</span><button onClick={() => void restoreSelected()}><ToolIcon name="restore" />Restore image</button></div>
            ) : selected?.status === "missing" ? (
              <div className="unavailable-stage"><ToolIcon name="image" /><strong>{selected.fileName} is missing</strong><span>Return it to {selected.relativePath}, then rescan.</span></div>
            ) : (
              <div className="unavailable-stage"><ToolIcon name="image" /><strong>{selected ? "Loading image…" : "Add images to the project folder"}</strong><span>{selected ? "Decoding locally" : "Rescan after adding JPEG, PNG, WebP, or TIFF files."}</span></div>
            )}
          </div>

          <footer className="editor-statusbar">
            <span className="file-status file-copy" title={selected?.relativePath}>
              <span>{selected?.fileName ?? "No image"}</span>
              {selected?.status === "active" && <button title="Copy full image path" aria-label="Copy full image path" onClick={() => void copySelectedPath()}><ToolIcon name="copy" /></button>}
            </span>
            <span>{activePosition >= 0 ? `${activePosition + 1} / ${activeImages.length}` : `0 / ${activeImages.length}`}</span>
            <span>{selected ? `${selected.width} × ${selected.height}` : "—"}</span>
            <span>{pointer ? `x ${Math.round(pointer.x)}  y ${Math.round(pointer.y)}` : "x —  y —"}</span>
            <span className="status-help">{project.taskType === "classification"
              ? "class key labels · arrows navigate · scroll to pan · pinch to zoom"
              : project.taskType === "detection"
                ? "drag to draw · hover to edit · ⌘ drag overlaps · Space drag pans"
                : `${shortcutLabel(shortcuts.edit)} edit · class key draws · scroll to pan · pinch to zoom`}</span>
          </footer>
        </section>

        <aside className="inspector">
          {project.taskType === "detection" || project.taskType === "segmentation" ? <>
            <div className="inspector-title">
              <div className="title-copy"><h2>Objects</h2><span>{annotations.length}</span></div>
              <button className="more-button" aria-label="Project options" aria-haspopup="true" aria-expanded={showProjectMenu} title="Project options" onClick={() => setShowProjectMenu((visible) => !visible)}><ToolIcon name="more" /></button>
              {showProjectMenu && <><button className="menu-scrim" aria-label="Close project options" onClick={() => setShowProjectMenu(false)} /><div className="project-menu">
                <strong>Import</strong>
                <button disabled={busy} onClick={() => void runExchange("labels")}><ToolIcon name="import" />Classes from labels.txt</button>
                {project.taskType === "detection" ? <>
                  <button disabled={busy} onClick={() => void runExchange("yolo-import")}><ToolIcon name="import" />YOLO boxes</button>
                  <button disabled={busy} onClick={() => void runExchange("coco-import")}><ToolIcon name="import" />COCO boxes</button>
                </> : <>
                  <button disabled={busy} onClick={() => void runExchange("yolo-polygon-import")}><ToolIcon name="import" />YOLO polygons</button>
                  <button disabled={busy} onClick={() => void runExchange("coco-polygon-import")}><ToolIcon name="import" />COCO polygons</button>
                </>}
                <strong>Export</strong>
                {project.taskType === "detection" ? <>
                  <button disabled={busy || classes.length === 0} onClick={() => void runExchange("yolo-export")}><ToolIcon name="export" />YOLO folder</button>
                  <button disabled={busy || classes.length === 0} onClick={() => void runExchange("coco-export")}><ToolIcon name="export" />COCO JSON</button>
                </> : <>
                  <button disabled={busy || classes.length === 0} onClick={() => void runExchange("yolo-polygon-export")}><ToolIcon name="export" />YOLO polygons</button>
                  <button disabled={busy || classes.length === 0} onClick={() => void runExchange("coco-polygon-export")}><ToolIcon name="export" />COCO JSON</button>
                </>}
                <strong>Keys</strong>
                <p><kbd>1</kbd>…<kbd>0</kbd> draw class</p>
                <p><kbd>{shortcutLabel(shortcuts.edit)}</kbd> edit objects</p>
                <p><kbd>{project.taskType === "detection" ? shortcutLabel(shortcuts.rectangle) : shortcutLabel(shortcuts.polygon)}</kbd> draw active class</p>
                {project.taskType === "detection" && <p><kbd>⌘ drag</kbd> draw through a box</p>}
                {project.taskType === "segmentation" && <p><kbd>↵</kbd> finish · <kbd>⌫</kbd> last point</p>}
                {project.taskType === "segmentation" && <p><kbd>{shortcutLabel(shortcuts.island)}</kbd> add island to selected</p>}
                <p><kbd>⌫</kbd> delete selected/last object</p>
                <p><kbd>D D</kbd> delete object-free image</p>
                <p><kbd>←</kbd><kbd>→</kbd> change image</p>
                <p><kbd>⌘ Z</kbd> undo</p>
                <button onClick={() => { setShowProjectMenu(false); setShowShortcutSettings(true); }}>Customize shortcuts…</button>
              </div></>}
            </div>
            {annotations.length > 0 ? <div className="annotation-list" aria-label="Objects on this image">
              {annotations.map((annotation, index) => {
                const classRecord = classes.find((item) => item.id === annotation.classId);
                return <button
                  className={`${annotation.id === selectedAnnotationId ? "active " : ""}${annotation.isVisible ? "" : "hidden"}`}
                  style={{ "--class-color": classRecord?.color ?? "#fff" } as React.CSSProperties}
                  onClick={() => { setSelectedAnnotationId(annotation.id); activateTool("select"); }}
                  title={annotation.isVisible ? `Edit ${classRecord?.name ?? "object"}` : `Edit hidden ${classRecord?.name ?? "object"}`}
                  key={annotation.id}
                ><small>{String(index + 1).padStart(2, "0")}</small><i /><span>{classRecord?.name ?? "Unknown class"}</span>{!annotation.isVisible && <em>hidden</em>}</button>;
              })}
            </div> : <p className="empty-objects">No objects on this image.</p>}
            {selectedAnnotation && (
              <section className="box-inspector" style={{ "--class-color": classes.find((item) => item.id === selectedAnnotation.classId)?.color ?? "#fff" } as React.CSSProperties}>
                <div className="box-selection-heading"><i /><strong>Selected {selectedAnnotation.kind === "bbox" ? "box" : "polygon"}</strong><button aria-label="Delete selected object" title="Delete object" onClick={() => void removeAnnotation()}><ToolIcon name="trash" /></button></div>
                <label>Class<select value={selectedAnnotation.classId} onChange={(event) => void changeSelectedAnnotation({ classId: event.target.value })}>{classes.map((item) => <option value={item.id} key={item.id}>{item.name}</option>)}</select></label>
                <label className="visibility-toggle"><input type="checkbox" checked={selectedAnnotation.isVisible} onChange={(event) => void changeSelectedAnnotation({ isVisible: event.target.checked })} /><ToolIcon name="eye" />Visible</label>
                <small>{annotationSummary(selectedAnnotation)}</small>
                {selectedAnnotation.kind === "polygon" && geometryWarnings(selectedAnnotation.geometry).map((warning) => <small className="geometry-warning" key={warning}>{warning}</small>)}
                {selectedAnnotation.kind === "polygon" && <button className={appendPolygonToId === selectedAnnotation.id ? "island-button active" : "island-button"} onClick={beginIsland} disabled={busy}>
                  <span>{appendPolygonToId === selectedAnnotation.id ? "Click to draw island" : "Add island"}</span><kbd>{shortcutLabel(shortcuts.island)}</kbd>
                </button>}
              </section>
            )}

            <section className="class-editor">
              <div className="panel-heading"><div><strong>Classes</strong><span>key → draw</span></div><button className="add-class-toggle" aria-label="Add class" aria-expanded={showAddClass} title="Add class" onClick={() => setShowAddClass((visible) => !visible)}>+</button></div>
              <div className="class-list">
                {classes.map((classRecord) => (
                  <div className={classRecord.id === activeClassId ? "class-row active" : "class-row"} key={classRecord.id} style={{ "--class-color": classRecord.color } as React.CSSProperties}>
                    {editingClassId === classRecord.id ? <div className="class-choice">
                      {classRecord.shortcut && <kbd>{classRecord.shortcut}</kbd>}<i />
                      <input autoFocus value={editingClassName} onChange={(event) => setEditingClassName(event.target.value)} onBlur={() => void commitClassRename()} onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); if (event.key === "Escape") setEditingClassId(null); }} />
                    </div> : <button className="class-choice" onClick={() => chooseClass(classRecord.id)} onDoubleClick={() => { setEditingClassId(classRecord.id); setEditingClassName(classRecord.name); }}>
                      {classRecord.shortcut && <kbd>{classRecord.shortcut}</kbd>}<i /><span>{classRecord.name}</span>
                    </button>}
                    <button className="remove-class" title={`Delete ${classRecord.name}`} disabled={busy} onClick={() => void removeClass(classRecord.id)}>×</button>
                  </div>
                ))}
                {classes.length === 0 && <p className="empty-classes">Add a class or load labels.txt before drawing.</p>}
              </div>
              {showAddClass && <form className="add-class" onSubmit={(event) => { event.preventDefault(); void addClass(); }}>
                <input value={newClassName} onChange={(event) => setNewClassName(event.target.value)} placeholder="New class name" aria-label="New class name" />
                <button disabled={!newClassName.trim() || busy}>Add</button>
              </form>}
            </section>
          </> : <>
            <div className="inspector-title">
              <div className="title-copy"><h2>Labels</h2><span>{labeledImageCount} / {currentProject.imageCount} labeled</span></div>
              <div className="inspector-actions">
                <button className="add-class-toggle" aria-label="Add class" aria-expanded={showAddClass} title="Add class" onClick={() => setShowAddClass((visible) => !visible)}>+</button>
                <button className="more-button" aria-label="Classification options" aria-haspopup="true" aria-expanded={showProjectMenu} title="Classification import and export" onClick={() => setShowProjectMenu((visible) => !visible)}><ToolIcon name="more" /></button>
              </div>
              {showProjectMenu && <><button className="menu-scrim" aria-label="Close project options" onClick={() => setShowProjectMenu(false)} /><div className="project-menu">
                <strong>Import</strong>
                <button disabled={busy} onClick={() => void runExchange("classification-csv-import")}><ToolIcon name="import" />Classification CSV</button>
                <button disabled={busy || project.classificationMode === "multi"} onClick={() => void runExchange("class-directories-import")}><ToolIcon name="import" />Class directories</button>
                <strong>Export</strong>
                <button disabled={busy} onClick={() => void runExchange("classification-csv-export")}><ToolIcon name="export" />Classification CSV</button>
                <button disabled={busy || classes.length === 0} onClick={() => void runExchange("class-directories-export")}><ToolIcon name="export" />Class directories</button>
                <strong>Keys</strong>
                <p><kbd>1</kbd>…<kbd>0</kbd> {project.classificationMode === "multi" ? "toggle label" : "label + advance"}</p>
                <p><kbd>D D</kbd> delete image</p>
                <p><kbd>←</kbd><kbd>→</kbd> change image</p>
                <p><kbd>⌘ Z</kbd> undo</p>
                <button onClick={() => { setShowProjectMenu(false); setShowShortcutSettings(true); }}>Customize shortcuts…</button>
              </div></>}
            </div>

            <p className="classification-hint">{project.classificationMode === "multi" ? "Choose every label that applies. Images stay in place." : "Choose one label to move the image into its class directory and advance."}</p>
            {showAddClass && <form className="add-class classification-add" onSubmit={(event) => { event.preventDefault(); void addClass(); }}>
              <input autoFocus value={newClassName} onChange={(event) => setNewClassName(event.target.value)} placeholder="New class name" aria-label="New class name" />
              <button disabled={!newClassName.trim() || busy}>Add</button>
            </form>}
            <div className="classification-list" aria-label="Image classes">
              {classes.map((classRecord) => {
                const checked = imageClassIds.includes(classRecord.id);
                return <div
                  className="classification-label-row"
                  style={{ "--class-color": classRecord.color } as React.CSSProperties}
                  key={classRecord.id}
                >
                  {editingClassId === classRecord.id ? <div className="inline-class-edit">
                    <kbd>{classRecord.shortcut ?? "—"}</kbd><i />
                    <input autoFocus value={editingClassName} onChange={(event) => setEditingClassName(event.target.value)} onBlur={() => void commitClassRename()} onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); if (event.key === "Escape") setEditingClassId(null); }} />
                    <button type="button" title={`Delete ${classRecord.name}`} disabled={busy} onMouseDown={(event) => event.preventDefault()} onClick={() => { setEditingClassId(null); void removeClass(classRecord.id); }}>Delete</button>
                  </div> : <>
                    <button
                      className={checked ? "classification-choice selected" : "classification-choice"}
                      aria-pressed={checked}
                      disabled={busy || selected?.status !== "active"}
                      onClick={() => void classifyImage(classRecord.id)}
                    >
                      <kbd>{classRecord.shortcut ?? "—"}</kbd><i /><span>{classRecord.name}</span><b>{checked ? "✓" : ""}</b>
                    </button>
                    <button className="edit-class-inline" title={`Rename or delete ${classRecord.name}`} onClick={() => { setEditingClassId(classRecord.id); setEditingClassName(classRecord.name); }}>Edit</button>
                  </>}
                </div>;
              })}
              {classes.length === 0 && <p className="empty-classes">Add the first class to begin labeling.</p>}
            </div>
            {imageClassIds.length > 0 && <button className="clear-classification" disabled={busy} onClick={() => void clearImageClassification()}>Clear {project.classificationMode === "multi" ? "labels" : "label"}</button>}

            {selected && <dl className="classification-meta"><div><dt>File</dt><dd title={selected.relativePath}>{selected.relativePath}</dd></div><div><dt>Dimensions</dt><dd>{selected.width} × {selected.height}</dd></div></dl>}
          </>}
          {project.taskType === "classification" && selected?.status === "active" && <button className="inspector-delete" disabled={busy} onClick={() => void moveSelectedToDeleted()}><ToolIcon name="trash" /><span><strong>Delete image</strong><small>Move it out of the dataset</small></span></button>}
          {selected?.status === "deleted" && <button className="inspector-restore" disabled={busy} onClick={() => void restoreSelected()}><ToolIcon name="restore" /><span><strong>Restore image</strong><small>Return it to its original folder</small></span></button>}
        </aside>
      </section>
      {showShortcutSettings && <ShortcutSettings shortcuts={shortcuts} onSave={commitShortcuts} onClose={() => setShowShortcutSettings(false)} />}
    </main>
  );
}

function toolLabel(tool: EditorTool): string {
  if (tool === "rectangle") return "Box";
  if (tool === "polygon") return "Polygon";
  return "Edit";
}

function toolShortcut(tool: EditorTool, shortcuts: ShortcutMap): string {
  if (tool === "rectangle") return shortcutLabel(shortcuts.rectangle);
  if (tool === "polygon") return shortcutLabel(shortcuts.polygon);
  return shortcutLabel(shortcuts.edit);
}

function annotationSummary(annotation: AnnotationRecord): string {
  if (annotation.kind === "bbox") {
    const geometry = annotation.geometry;
    return `${Math.round(geometry.x)}, ${Math.round(geometry.y)} · ${Math.round(geometry.width)} × ${Math.round(geometry.height)} px`;
  }
  const contours = annotation.geometry.polygons.length;
  const vertices = annotation.geometry.polygons.reduce((sum, polygon) => sum + polygon.length, 0);
  return `${vertices} vertices · ${contours} ${contours === 1 ? "contour" : "contours"} · ${Math.round(geometryArea(annotation.geometry))} px²`;
}

function panelPreference(key: string, fallback: boolean, legacyKey?: string): boolean {
  const stored = localStorage.getItem(key) ?? (legacyKey ? localStorage.getItem(legacyKey) : null);
  return stored === null ? fallback : stored !== "hidden";
}

function isTypingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement
    || target instanceof HTMLTextAreaElement
    || (target instanceof HTMLElement && target.isContentEditable);
}
