export type TaskType = "detection" | "segmentation" | "classification" | "keypoints";
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

/** v: 0 unlabeled, 1 labeled but occluded, 2 visible (COCO / YOLO-pose flags). */
export interface Keypoint {
  x: number;
  y: number;
  v: number;
}

export interface KeypointGeometry {
  bbox: BboxGeometry;
  keypoints: Keypoint[];
}

/** The ordered points every object in a keypoint project carries. Skeleton pairs are zero-based. */
export interface KeypointSchema {
  names: string[];
  skeleton: [number, number][];
}

/** The keypoint slot the next canvas click places; `single` ends placement after that click. */
export interface KeypointPlacement {
  annotationId: string;
  index: number;
  single?: boolean;
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

export interface KeypointAnnotationRecord extends AnnotationBase {
  kind: "keypoints";
  geometry: KeypointGeometry;
}

export type AnnotationRecord = BboxAnnotationRecord | PolygonAnnotationRecord | KeypointAnnotationRecord;

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
