mod classification;
mod detection;
mod migrations;
mod project;
mod recents;
mod segmentation;
mod workspace;

use classification::{ClassificationReport, ClassificationState};
use detection::{
    AnnotationRecord, BboxDraft, BboxGeometry, ClassRecord, DetectionState, ImportReport,
};
use project::{ImageRecord, ProjectSummary};
use recents::RecentProject;
use segmentation::{PolygonAnnotationRecord, PolygonDraft, PolygonGeometry};
use tauri::Manager;
use workspace::{HistoryState, ViewportSnapshot};

#[tauri::command(rename_all = "camelCase")]
async fn create_project(
    app: tauri::AppHandle,
    root_path: String,
    name: String,
    task_type: String,
    classification_mode: Option<String>,
) -> Result<ProjectSummary, String> {
    let app_for_recent = app.clone();
    let project = tauri::async_runtime::spawn_blocking(move || {
        project::create(
            &root_path,
            &name,
            &task_type,
            classification_mode.as_deref(),
        )
    })
    .await
    .map_err(|error| format!("Project creation stopped unexpectedly: {error}"))??;
    remember_project(&app_for_recent, &project)?;
    Ok(project)
}

#[tauri::command(rename_all = "camelCase")]
async fn open_project(app: tauri::AppHandle, root_path: String) -> Result<ProjectSummary, String> {
    let app_for_recent = app.clone();
    let project = tauri::async_runtime::spawn_blocking(move || project::open(&root_path))
        .await
        .map_err(|error| format!("Opening the project stopped unexpectedly: {error}"))??;
    remember_project(&app_for_recent, &project)?;
    Ok(project)
}

#[tauri::command(rename_all = "camelCase")]
async fn rescan_project(root_path: String) -> Result<ProjectSummary, String> {
    tauri::async_runtime::spawn_blocking(move || project::rescan(&root_path))
        .await
        .map_err(|error| format!("Image indexing stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn get_project_summary(root_path: String) -> Result<ProjectSummary, String> {
    tauri::async_runtime::spawn_blocking(move || project::summary(&root_path))
        .await
        .map_err(|error| format!("Loading the project summary stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn list_project_images(
    root_path: String,
    limit: u32,
    offset: u32,
) -> Result<Vec<ImageRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || project::list_images(&root_path, limit, offset))
        .await
        .map_err(|error| format!("Loading images stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn get_thumbnail(root_path: String, image_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || project::thumbnail_data_url(&root_path, &image_id))
        .await
        .map_err(|error| format!("Thumbnail generation stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn read_image_file(
    root_path: String,
    image_id: String,
) -> Result<tauri::ipc::Response, String> {
    let bytes =
        tauri::async_runtime::spawn_blocking(move || project::image_bytes(&root_path, &image_id))
            .await
            .map_err(|error| format!("Image loading stopped unexpectedly: {error}"))??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command(rename_all = "camelCase")]
async fn save_workspace_state(
    root_path: String,
    image_id: String,
    viewport: Option<ViewportSnapshot>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        workspace::save_viewport(&root_path, &image_id, viewport)
    })
    .await
    .map_err(|error| format!("Workspace autosave stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn load_viewport_state(
    root_path: String,
    image_id: String,
) -> Result<Option<ViewportSnapshot>, String> {
    tauri::async_runtime::spawn_blocking(move || workspace::load_viewport(&root_path, &image_id))
        .await
        .map_err(|error| format!("Viewport recovery stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn delete_project_image(
    root_path: String,
    image_id: String,
) -> Result<ProjectSummary, String> {
    tauri::async_runtime::spawn_blocking(move || workspace::delete_image(&root_path, &image_id))
        .await
        .map_err(|error| format!("Moving the image stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn restore_project_image(
    root_path: String,
    image_id: String,
) -> Result<ProjectSummary, String> {
    tauri::async_runtime::spawn_blocking(move || workspace::restore_image(&root_path, &image_id))
        .await
        .map_err(|error| format!("Restoring the image stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn undo_project_operation(root_path: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || workspace::undo(&root_path))
        .await
        .map_err(|error| format!("Undo stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn redo_project_operation(root_path: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || workspace::redo(&root_path))
        .await
        .map_err(|error| format!("Redo stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn get_history_state(root_path: String) -> Result<HistoryState, String> {
    tauri::async_runtime::spawn_blocking(move || workspace::history_state(&root_path))
        .await
        .map_err(|error| format!("History inspection stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn get_classification_state(
    root_path: String,
    image_id: Option<String>,
) -> Result<ClassificationState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        classification::state(&root_path, image_id.as_deref())
    })
    .await
    .map_err(|error| format!("Loading classification stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn set_image_classification(
    root_path: String,
    image_id: String,
    class_ids: Vec<String>,
) -> Result<ClassificationState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        classification::set_image_classes(&root_path, &image_id, class_ids)
    })
    .await
    .map_err(|error| format!("Classifying the image stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn import_classification_csv(
    root_path: String,
    source_path: String,
) -> Result<ClassificationReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        classification::import_csv(&root_path, &source_path)
    })
    .await
    .map_err(|error| format!("Classification CSV import stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn export_classification_csv(
    root_path: String,
    destination_path: String,
) -> Result<ClassificationReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        classification::export_csv(&root_path, &destination_path)
    })
    .await
    .map_err(|error| format!("Classification CSV export stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn import_class_directories(
    root_path: String,
    source_path: String,
) -> Result<ClassificationReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        classification::import_class_directories(&root_path, &source_path)
    })
    .await
    .map_err(|error| format!("Class-directory import stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn export_class_directories(
    root_path: String,
    destination_path: String,
) -> Result<ClassificationReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        classification::export_class_directories(&root_path, &destination_path)
    })
    .await
    .map_err(|error| format!("Class-directory export stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn get_detection_state(root_path: String) -> Result<DetectionState, String> {
    tauri::async_runtime::spawn_blocking(move || detection::state(&root_path))
        .await
        .map_err(|error| format!("Loading classes stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn create_detection_class(root_path: String, name: String) -> Result<ClassRecord, String> {
    tauri::async_runtime::spawn_blocking(move || detection::create_class(&root_path, &name))
        .await
        .map_err(|error| format!("Creating the class stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn rename_detection_class(
    root_path: String,
    class_id: String,
    name: String,
) -> Result<ClassRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        detection::rename_class(&root_path, &class_id, &name)
    })
    .await
    .map_err(|error| format!("Renaming the class stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn delete_detection_class(
    root_path: String,
    class_id: String,
) -> Result<DetectionState, String> {
    tauri::async_runtime::spawn_blocking(move || detection::delete_class(&root_path, &class_id))
        .await
        .map_err(|error| format!("Deleting the class stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn set_active_detection_class(root_path: String, class_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || detection::set_active_class(&root_path, &class_id))
        .await
        .map_err(|error| format!("Saving the active class stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn list_image_annotations(
    root_path: String,
    image_id: String,
) -> Result<Vec<AnnotationRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || detection::list_annotations(&root_path, &image_id))
        .await
        .map_err(|error| format!("Loading annotations stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn create_bbox_annotation(
    root_path: String,
    image_id: String,
    class_id: String,
    geometry: BboxGeometry,
) -> Result<AnnotationRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        detection::create_annotation(&root_path, &image_id, &class_id, geometry)
    })
    .await
    .map_err(|error| format!("Saving the box stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn update_bbox_annotation(
    root_path: String,
    annotation_id: String,
    class_id: String,
    geometry: BboxGeometry,
    is_visible: bool,
) -> Result<AnnotationRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        detection::update_annotation(&root_path, &annotation_id, &class_id, geometry, is_visible)
    })
    .await
    .map_err(|error| format!("Updating the box stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn delete_bbox_annotation(root_path: String, annotation_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        detection::delete_annotation(&root_path, &annotation_id)
    })
    .await
    .map_err(|error| format!("Deleting the box stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn save_bbox_draft(
    root_path: String,
    image_id: String,
    draft: Option<BboxDraft>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        detection::save_draft(&root_path, draft, &image_id)
    })
    .await
    .map_err(|error| format!("Saving the box draft stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn load_bbox_draft(root_path: String, image_id: String) -> Result<Option<BboxDraft>, String> {
    tauri::async_runtime::spawn_blocking(move || detection::load_draft(&root_path, &image_id))
        .await
        .map_err(|error| format!("Loading the box draft stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn import_detection_labels(
    root_path: String,
    source_path: String,
) -> Result<DetectionState, String> {
    tauri::async_runtime::spawn_blocking(move || detection::import_labels(&root_path, &source_path))
        .await
        .map_err(|error| format!("Loading labels stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn import_yolo_annotations(
    root_path: String,
    source_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || detection::import_yolo(&root_path, &source_path))
        .await
        .map_err(|error| format!("YOLO import stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn export_yolo_annotations(
    root_path: String,
    destination_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        detection::export_yolo(&root_path, &destination_path)
    })
    .await
    .map_err(|error| format!("YOLO export stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn import_coco_annotations(
    root_path: String,
    source_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || detection::import_coco(&root_path, &source_path))
        .await
        .map_err(|error| format!("COCO import stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn export_coco_annotations(
    root_path: String,
    destination_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        detection::export_coco(&root_path, &destination_path)
    })
    .await
    .map_err(|error| format!("COCO export stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn list_polygon_annotations(
    root_path: String,
    image_id: String,
) -> Result<Vec<PolygonAnnotationRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::list_annotations(&root_path, &image_id)
    })
    .await
    .map_err(|error| format!("Loading polygons stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn create_polygon_annotation(
    root_path: String,
    image_id: String,
    class_id: String,
    geometry: PolygonGeometry,
) -> Result<PolygonAnnotationRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::create_annotation(&root_path, &image_id, &class_id, geometry)
    })
    .await
    .map_err(|error| format!("Saving the polygon stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn update_polygon_annotation(
    root_path: String,
    annotation_id: String,
    class_id: String,
    geometry: PolygonGeometry,
    is_visible: bool,
) -> Result<PolygonAnnotationRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::update_annotation(&root_path, &annotation_id, &class_id, geometry, is_visible)
    })
    .await
    .map_err(|error| format!("Updating the polygon stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn append_polygon_island(
    root_path: String,
    annotation_id: String,
    points: Vec<segmentation::PolygonPoint>,
) -> Result<PolygonAnnotationRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::append_island(&root_path, &annotation_id, points)
    })
    .await
    .map_err(|error| format!("Adding the polygon island stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn delete_polygon_annotation(root_path: String, annotation_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::delete_annotation(&root_path, &annotation_id)
    })
    .await
    .map_err(|error| format!("Deleting the polygon stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn save_polygon_draft(
    root_path: String,
    image_id: String,
    draft: Option<PolygonDraft>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::save_draft(&root_path, draft, &image_id)
    })
    .await
    .map_err(|error| format!("Saving the polygon draft stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn load_polygon_draft(
    root_path: String,
    image_id: String,
) -> Result<Option<PolygonDraft>, String> {
    tauri::async_runtime::spawn_blocking(move || segmentation::load_draft(&root_path, &image_id))
        .await
        .map_err(|error| format!("Loading the polygon draft stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn import_coco_polygons(
    root_path: String,
    source_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::import_coco(&root_path, &source_path)
    })
    .await
    .map_err(|error| format!("COCO polygon import stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn import_yolo_polygons(
    root_path: String,
    source_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::import_yolo(&root_path, &source_path)
    })
    .await
    .map_err(|error| format!("YOLO polygon import stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn export_coco_polygons(
    root_path: String,
    destination_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::export_coco(&root_path, &destination_path)
    })
    .await
    .map_err(|error| format!("COCO polygon export stopped unexpectedly: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn export_yolo_polygons(
    root_path: String,
    destination_path: String,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        segmentation::export_yolo(&root_path, &destination_path)
    })
    .await
    .map_err(|error| format!("YOLO polygon export stopped unexpectedly: {error}"))?
}

#[tauri::command]
fn list_recent_projects(app: tauri::AppHandle) -> Result<Vec<RecentProject>, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate app data storage: {error}"))?;
    recents::list(&directory)
}

fn remember_project(app: &tauri::AppHandle, project: &ProjectSummary) -> Result<(), String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate app data storage: {error}"))?;
    recents::remember(&directory, project)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            create_project,
            open_project,
            rescan_project,
            get_project_summary,
            list_project_images,
            get_thumbnail,
            read_image_file,
            save_workspace_state,
            load_viewport_state,
            delete_project_image,
            restore_project_image,
            undo_project_operation,
            redo_project_operation,
            get_history_state,
            get_classification_state,
            set_image_classification,
            import_classification_csv,
            export_classification_csv,
            import_class_directories,
            export_class_directories,
            get_detection_state,
            create_detection_class,
            rename_detection_class,
            delete_detection_class,
            set_active_detection_class,
            list_image_annotations,
            create_bbox_annotation,
            update_bbox_annotation,
            delete_bbox_annotation,
            save_bbox_draft,
            load_bbox_draft,
            import_detection_labels,
            import_yolo_annotations,
            export_yolo_annotations,
            import_coco_annotations,
            export_coco_annotations,
            list_polygon_annotations,
            create_polygon_annotation,
            update_polygon_annotation,
            append_polygon_island,
            delete_polygon_annotation,
            save_polygon_draft,
            load_polygon_draft,
            import_yolo_polygons,
            import_coco_polygons,
            export_coco_polygons,
            export_yolo_polygons,
            list_recent_projects
        ])
        .run(tauri::generate_context!())
        .expect("error while running yalt");
}
