import { invoke } from "@tauri-apps/api/core";
import type {
  AnnotationRecord,
  BboxDraft,
  BboxGeometry,
  ClassRecord,
  ClassificationMode,
  ClassificationReport,
  ClassificationState,
  ImageRecord,
  HistoryState,
  DetectionState,
  ImportReport,
  PolygonAnnotationRecord,
  PolygonDraft,
  PolygonGeometry,
  PolygonPoint,
  ProjectSummary,
  RecentProject,
  TaskType,
  ViewportSnapshot,
} from "./types";

export function getClassificationState(rootPath: string, imageId: string | null): Promise<ClassificationState> {
  return invoke("get_classification_state", { rootPath, imageId });
}

export function setImageClassification(rootPath: string, imageId: string, classIds: string[]): Promise<ClassificationState> {
  return invoke("set_image_classification", { rootPath, imageId, classIds });
}

export function importClassificationCsv(rootPath: string, sourcePath: string): Promise<ClassificationReport> {
  return invoke("import_classification_csv", { rootPath, sourcePath });
}

export function exportClassificationCsv(rootPath: string, destinationPath: string): Promise<ClassificationReport> {
  return invoke("export_classification_csv", { rootPath, destinationPath });
}

export function importClassDirectories(rootPath: string, sourcePath: string): Promise<ClassificationReport> {
  return invoke("import_class_directories", { rootPath, sourcePath });
}

export function exportClassDirectories(rootPath: string, destinationPath: string): Promise<ClassificationReport> {
  return invoke("export_class_directories", { rootPath, destinationPath });
}

export function listRecentProjects(): Promise<RecentProject[]> {
  return invoke("list_recent_projects");
}

export function createProject(input: {
  rootPath: string;
  name: string;
  taskType: TaskType;
  classificationMode: ClassificationMode | null;
}): Promise<ProjectSummary> {
  return invoke("create_project", input);
}

export function openProject(rootPath: string): Promise<ProjectSummary> {
  return invoke("open_project", { rootPath });
}

export function removeProject(rootPath: string): Promise<void> {
  return invoke("remove_project", { rootPath });
}

export function rescanProject(rootPath: string): Promise<ProjectSummary> {
  return invoke("rescan_project", { rootPath });
}

export function getProjectSummary(rootPath: string): Promise<ProjectSummary> {
  return invoke("get_project_summary", { rootPath });
}

export async function listProjectImages(rootPath: string): Promise<ImageRecord[]> {
  const images: ImageRecord[] = [];
  const pageSize = 1000;
  for (let offset = 0; ; offset += pageSize) {
    const page = await invoke<ImageRecord[]>("list_project_images", {
      rootPath,
      limit: pageSize,
      offset,
    });
    images.push(...page);
    if (page.length < pageSize) return images;
  }
}

export function getThumbnail(rootPath: string, imageId: string): Promise<string> {
  return invoke("get_thumbnail", { rootPath, imageId });
}

export function readImageFile(rootPath: string, imageId: string): Promise<ArrayBuffer | number[]> {
  return invoke("read_image_file", { rootPath, imageId });
}

export function saveWorkspaceState(
  rootPath: string,
  imageId: string,
  viewport: ViewportSnapshot | null,
): Promise<void> {
  return invoke("save_workspace_state", { rootPath, imageId, viewport });
}

export function loadViewportState(rootPath: string, imageId: string): Promise<ViewportSnapshot | null> {
  return invoke("load_viewport_state", { rootPath, imageId });
}

export function deleteProjectImage(rootPath: string, imageId: string): Promise<ProjectSummary> {
  return invoke("delete_project_image", { rootPath, imageId });
}

export function restoreProjectImage(rootPath: string, imageId: string): Promise<ProjectSummary> {
  return invoke("restore_project_image", { rootPath, imageId });
}

export function undoProjectOperation(rootPath: string): Promise<string | null> {
  return invoke("undo_project_operation", { rootPath });
}

export function redoProjectOperation(rootPath: string): Promise<string | null> {
  return invoke("redo_project_operation", { rootPath });
}

export function getHistoryState(rootPath: string): Promise<HistoryState> {
  return invoke("get_history_state", { rootPath });
}

export function getDetectionState(rootPath: string): Promise<DetectionState> {
  return invoke("get_detection_state", { rootPath });
}

export function createDetectionClass(rootPath: string, name: string): Promise<ClassRecord> {
  return invoke("create_detection_class", { rootPath, name });
}

export function renameDetectionClass(rootPath: string, classId: string, name: string): Promise<ClassRecord> {
  return invoke("rename_detection_class", { rootPath, classId, name });
}

export function deleteDetectionClass(rootPath: string, classId: string): Promise<DetectionState> {
  return invoke("delete_detection_class", { rootPath, classId });
}

export function setActiveDetectionClass(rootPath: string, classId: string): Promise<void> {
  return invoke("set_active_detection_class", { rootPath, classId });
}

export function listImageAnnotations(rootPath: string, imageId: string): Promise<AnnotationRecord[]> {
  return invoke("list_image_annotations", { rootPath, imageId });
}

export function createBboxAnnotation(rootPath: string, imageId: string, classId: string, geometry: BboxGeometry): Promise<AnnotationRecord> {
  return invoke("create_bbox_annotation", { rootPath, imageId, classId, geometry });
}

export function updateBboxAnnotation(rootPath: string, annotationId: string, classId: string, geometry: BboxGeometry, isVisible: boolean): Promise<AnnotationRecord> {
  return invoke("update_bbox_annotation", { rootPath, annotationId, classId, geometry, isVisible });
}

export function deleteBboxAnnotation(rootPath: string, annotationId: string): Promise<void> {
  return invoke("delete_bbox_annotation", { rootPath, annotationId });
}

export function saveBboxDraft(rootPath: string, imageId: string, draft: BboxDraft | null): Promise<void> {
  return invoke("save_bbox_draft", { rootPath, imageId, draft });
}

export function loadBboxDraft(rootPath: string, imageId: string): Promise<BboxDraft | null> {
  return invoke("load_bbox_draft", { rootPath, imageId });
}

export function importDetectionLabels(rootPath: string, sourcePath: string): Promise<DetectionState> {
  return invoke("import_detection_labels", { rootPath, sourcePath });
}

export function importYoloAnnotations(rootPath: string, sourcePath: string): Promise<ImportReport> {
  return invoke("import_yolo_annotations", { rootPath, sourcePath });
}

export function exportYoloAnnotations(rootPath: string, destinationPath: string): Promise<ImportReport> {
  return invoke("export_yolo_annotations", { rootPath, destinationPath });
}

export function importCocoAnnotations(rootPath: string, sourcePath: string): Promise<ImportReport> {
  return invoke("import_coco_annotations", { rootPath, sourcePath });
}

export function exportCocoAnnotations(rootPath: string, destinationPath: string): Promise<ImportReport> {
  return invoke("export_coco_annotations", { rootPath, destinationPath });
}

export function listPolygonAnnotations(rootPath: string, imageId: string): Promise<PolygonAnnotationRecord[]> {
  return invoke("list_polygon_annotations", { rootPath, imageId });
}

export function createPolygonAnnotation(rootPath: string, imageId: string, classId: string, geometry: PolygonGeometry): Promise<PolygonAnnotationRecord> {
  return invoke("create_polygon_annotation", { rootPath, imageId, classId, geometry });
}

export function updatePolygonAnnotation(rootPath: string, annotationId: string, classId: string, geometry: PolygonGeometry, isVisible: boolean): Promise<PolygonAnnotationRecord> {
  return invoke("update_polygon_annotation", { rootPath, annotationId, classId, geometry, isVisible });
}

export function appendPolygonIsland(rootPath: string, annotationId: string, points: PolygonPoint[]): Promise<PolygonAnnotationRecord> {
  return invoke("append_polygon_island", { rootPath, annotationId, points });
}

export function deletePolygonAnnotation(rootPath: string, annotationId: string): Promise<void> {
  return invoke("delete_polygon_annotation", { rootPath, annotationId });
}

export function savePolygonDraft(rootPath: string, imageId: string, draft: PolygonDraft | null): Promise<void> {
  return invoke("save_polygon_draft", { rootPath, imageId, draft });
}

export function loadPolygonDraft(rootPath: string, imageId: string): Promise<PolygonDraft | null> {
  return invoke("load_polygon_draft", { rootPath, imageId });
}

export function importCocoPolygons(rootPath: string, sourcePath: string): Promise<ImportReport> {
  return invoke("import_coco_polygons", { rootPath, sourcePath });
}

export function importYoloPolygons(rootPath: string, sourcePath: string): Promise<ImportReport> {
  return invoke("import_yolo_polygons", { rootPath, sourcePath });
}

export function exportCocoPolygons(rootPath: string, destinationPath: string): Promise<ImportReport> {
  return invoke("export_coco_polygons", { rootPath, destinationPath });
}

export function exportYoloPolygons(rootPath: string, destinationPath: string): Promise<ImportReport> {
  return invoke("export_yolo_polygons", { rootPath, destinationPath });
}
