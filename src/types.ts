export type TaskType = "detection" | "segmentation" | "classification";
export type ClassificationMode = "single" | "multi";

export interface ProjectSummary {
  id: string;
  name: string;
  taskType: TaskType;
  classificationMode: ClassificationMode | null;
  rootPath: string;
  createdAtMs: number;
  updatedAtMs: number;
  imageCount: number;
  missingImageCount: number;
  deletedImageCount: number;
  lastImageId: string | null;
}

export interface RecentProject {
  name: string;
  rootPath: string;
  taskType: TaskType;
  lastOpenedAtMs: number;
  available: boolean;
  previewImageId: string | null;
}

export interface ImageRecord {
  id: string;
  relativePath: string;
  fileName: string;
  width: number;
  height: number;
  status: "active" | "missing" | "deleted";
  orientation: number;
}

export interface ViewportSnapshot {
  scale: number;
  centerX: number;
  centerY: number;
}

export interface HistoryState {
  canUndo: boolean;
  canRedo: boolean;
}

export type EditorTool = "select" | "rectangle" | "polygon";

export interface ClassRecord {
  id: string;
  name: string;
  position: number;
  color: string;
  shortcut: string | null;
}

export interface BboxGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PolygonPoint {
  x: number;
  y: number;
}

export interface PolygonGeometry {
  polygons: PolygonPoint[][];
}

interface AnnotationBase {
  id: string;
  imageId: string;
  classId: string;
  isVisible: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface BboxAnnotationRecord extends AnnotationBase {
  kind: "bbox";
  geometry: BboxGeometry;
}

export interface PolygonAnnotationRecord extends AnnotationBase {
  kind: "polygon";
  geometry: PolygonGeometry;
}

export type AnnotationRecord = BboxAnnotationRecord | PolygonAnnotationRecord;

export interface BboxDraft {
  imageId: string;
  classId: string;
  x: number;
  y: number;
}

export interface PolygonDraft {
  imageId: string;
  classId: string;
  points: PolygonPoint[];
  annotationId?: string | null;
}

export type AnnotationDraft = BboxDraft | PolygonDraft;

export interface DetectionState {
  classes: ClassRecord[];
  activeClassId: string | null;
}

export interface ImportReport {
  annotations: number;
  matchedImages: number;
  skipped: number;
  classes: number;
  message: string;
}

export interface ClassificationState {
  classes: ClassRecord[];
  imageClassIds: string[];
  labeledImageCount: number;
}

export interface ClassificationReport {
  assignments: number;
  matchedImages: number;
  skipped: number;
  classes: number;
  message: string;
}
