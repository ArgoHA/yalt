use crate::project;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

type IndexedImage = (String, String, String, u32, u32);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClassRecord {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub color: String,
    pub shortcut: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BboxGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationRecord {
    pub id: String,
    pub image_id: String,
    pub class_id: String,
    pub kind: String,
    pub geometry: BboxGeometry,
    pub is_visible: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BboxDraft {
    pub image_id: String,
    pub class_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionState {
    pub classes: Vec<ClassRecord>,
    pub active_class_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub annotations: usize,
    pub matched_images: usize,
    pub skipped: usize,
    pub classes: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnnotationOperation {
    operation_type: String,
    before: Vec<AnnotationRecord>,
    after: Vec<AnnotationRecord>,
}

pub fn state(root_path: &str) -> Result<DetectionState, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_spatial_task(&connection)?;
    let classes = classes_from_connection(&connection)?;
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key = 'active_class_id'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not restore the active class: {error}"))?;
    let active_class_id = stored
        .filter(|id| classes.iter().any(|class| class.id == *id))
        .or_else(|| classes.first().map(|class| class.id.clone()));
    Ok(DetectionState {
        classes,
        active_class_id,
    })
}

pub fn create_class(root_path: &str, name: &str) -> Result<ClassRecord, String> {
    let name = valid_class_name(name)?;
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_spatial_task(&connection)?;
    let position: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM classes",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not place the class: {error}"))?;
    let record = ClassRecord {
        id: Uuid::new_v4().to_string(),
        name,
        position,
        color: spectrum_color(position as usize),
        shortcut: shortcut_for(position),
    };
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not add the class: {error}"))?;
    insert_class(&transaction, &record, now)?;
    transaction.execute(
        "INSERT INTO settings (key, value) VALUES ('active_class_id', ?1) ON CONFLICT(key) DO NOTHING",
        [&record.id],
    ).map_err(|error| format!("Could not set the first active class: {error}"))?;
    touch_project(&transaction, now)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not save the class: {error}"))?;
    Ok(record)
}

pub fn rename_class(root_path: &str, class_id: &str, name: &str) -> Result<ClassRecord, String> {
    let name = valid_class_name(name)?;
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_spatial_task(&connection)?;
    let now = project::now_ms()?;
    connection
        .execute(
            "UPDATE classes SET name = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![name, now, class_id],
        )
        .map_err(|error| format!("Could not rename the class: {error}"))?;
    connection
        .execute("UPDATE project SET updated_at_ms = ?1", [now])
        .map_err(|error| format!("Could not update project state: {error}"))?;
    class_by_id(&connection, class_id)?.ok_or_else(|| "The class no longer exists.".to_owned())
}

pub fn delete_class(root_path: &str, class_id: &str) -> Result<DetectionState, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_spatial_task(&connection)?;
    let previous_active: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key = 'active_class_id'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read the active class: {error}"))?;
    let used: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM annotations WHERE class_id = ?1 UNION SELECT 1 FROM drafts WHERE json_extract(payload_json, '$.classId') = ?1 UNION SELECT 1 FROM image_classes WHERE class_id = ?1)",
        [class_id], |row| row.get(0),
    ).map_err(|error| format!("Could not inspect class usage: {error}"))?;
    if used {
        return Err("This class is used by annotations. Relabel or delete them first.".to_owned());
    }
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not remove the class: {error}"))?;
    let changed = transaction
        .execute("DELETE FROM classes WHERE id = ?1", [class_id])
        .map_err(|error| format!("Could not remove the class: {error}"))?;
    if changed == 0 {
        return Err("The class no longer exists.".to_owned());
    }
    compact_class_positions(&transaction)?;
    let first: Option<String> = transaction
        .query_row(
            "SELECT id FROM classes ORDER BY position LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not choose an active class: {error}"))?;
    let next_active = previous_active.filter(|id| id != class_id).or(first);
    match next_active {
        Some(id) => {
            transaction.execute("INSERT INTO settings (key, value) VALUES ('active_class_id', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [id]).map_err(|error| format!("Could not save the active class: {error}"))?;
        }
        None => {
            transaction
                .execute("DELETE FROM settings WHERE key = 'active_class_id'", [])
                .map_err(|error| format!("Could not clear the active class: {error}"))?;
        }
    }
    touch_project(&transaction, project::now_ms()?)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not finish removing the class: {error}"))?;
    state(root_path)
}

pub fn set_active_class(root_path: &str, class_id: &str) -> Result<(), String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_spatial_task(&connection)?;
    if class_by_id(&connection, class_id)?.is_none() {
        return Err("The class no longer exists.".to_owned());
    }
    connection.execute(
        "INSERT INTO settings (key, value) VALUES ('active_class_id', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [class_id],
    ).map(|_| ()).map_err(|error| format!("Could not save the active class: {error}"))
}

pub fn list_annotations(root_path: &str, image_id: &str) -> Result<Vec<AnnotationRecord>, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    annotations_for_image(&connection, image_id)
}

pub fn create_annotation(
    root_path: &str,
    image_id: &str,
    class_id: &str,
    geometry: BboxGeometry,
) -> Result<AnnotationRecord, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    validate_annotation(&connection, image_id, class_id, geometry)?;
    let now = project::now_ms()?;
    let record = AnnotationRecord {
        id: Uuid::new_v4().to_string(),
        image_id: image_id.to_owned(),
        class_id: class_id.to_owned(),
        kind: "bbox".to_owned(),
        geometry,
        is_visible: true,
        created_at_ms: now,
        updated_at_ms: now,
    };
    commit_annotation_change(&mut connection, vec![], vec![record.clone()])?;
    clear_draft_in_connection(&connection, image_id)?;
    Ok(record)
}

pub fn update_annotation(
    root_path: &str,
    annotation_id: &str,
    class_id: &str,
    geometry: BboxGeometry,
    is_visible: bool,
) -> Result<AnnotationRecord, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    let before = annotation_by_id(&connection, annotation_id)?
        .ok_or_else(|| "The box no longer exists.".to_owned())?;
    validate_annotation(&connection, &before.image_id, class_id, geometry)?;
    let mut after = before.clone();
    after.class_id = class_id.to_owned();
    after.geometry = geometry;
    after.is_visible = is_visible;
    after.updated_at_ms = project::now_ms()?;
    commit_annotation_change(&mut connection, vec![before], vec![after.clone()])?;
    Ok(after)
}

pub fn delete_annotation(root_path: &str, annotation_id: &str) -> Result<(), String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    let before = annotation_by_id(&connection, annotation_id)?
        .ok_or_else(|| "The box no longer exists.".to_owned())?;
    commit_annotation_change(&mut connection, vec![before], vec![])
}

pub fn save_draft(root_path: &str, draft: Option<BboxDraft>, image_id: &str) -> Result<(), String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    if let Some(draft) = draft {
        if draft.image_id != image_id
            || !draft.x.is_finite()
            || !draft.y.is_finite()
            || class_by_id(&connection, &draft.class_id)?.is_none()
        {
            return Err("The box draft is invalid.".to_owned());
        }
        let payload = serde_json::to_string(&draft)
            .map_err(|error| format!("Could not encode the box draft: {error}"))?;
        connection.execute(
            "INSERT INTO drafts (image_id, kind, payload_json, updated_at_ms) VALUES (?1, 'bbox', ?2, ?3) ON CONFLICT(image_id) DO UPDATE SET kind = 'bbox', payload_json = excluded.payload_json, updated_at_ms = excluded.updated_at_ms",
            params![image_id, payload, project::now_ms()?],
        ).map(|_| ()).map_err(|error| format!("Could not autosave the box draft: {error}"))
    } else {
        clear_draft_in_connection(&connection, image_id)
    }
}

pub fn load_draft(root_path: &str, image_id: &str) -> Result<Option<BboxDraft>, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload_json FROM drafts WHERE image_id = ?1 AND kind = 'bbox'",
            [image_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not restore the box draft: {error}"))?;
    payload
        .map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| format!("Could not understand the saved box draft: {error}"))
        })
        .transpose()
}

pub fn import_labels(root_path: &str, labels_path: &str) -> Result<DetectionState, String> {
    let text = fs::read_to_string(labels_path)
        .map_err(|error| format!("Could not read labels.txt: {error}"))?;
    let names = label_names(&text)?;
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_spatial_task(&connection)?;
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not load labels: {error}"))?;
    let existing = {
        let mut statement = transaction
            .prepare("SELECT id, name, position FROM classes ORDER BY position")
            .map_err(|error| format!("Could not inspect existing labels: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| format!("Could not inspect existing labels: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Could not read existing labels: {error}"))?
    };
    // Move existing names out of the unique namespace first, so swapping two
    // YOLO indices (for example person/car → car/person) is transactional.
    for (id, _, _) in &existing {
        transaction
            .execute(
                "UPDATE classes SET name = ?1 WHERE id = ?2",
                params![format!("__yalt_import_{id}"), id],
            )
            .map_err(|error| format!("Could not prepare labels for import: {error}"))?;
    }
    for (position, name) in names.iter().enumerate() {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM classes WHERE position = ?1",
                [position as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Could not inspect labels: {error}"))?;
        if let Some(id) = existing {
            transaction
                .execute(
                    "UPDATE classes SET name = ?1, updated_at_ms = ?2 WHERE id = ?3",
                    params![name, now, id],
                )
                .map_err(|error| format!("Could not update label {name}: {error}"))?;
        } else {
            let record = ClassRecord {
                id: Uuid::new_v4().to_string(),
                name: name.clone(),
                position: position as i64,
                color: spectrum_color(position),
                shortcut: shortcut_for(position as i64),
            };
            insert_class(&transaction, &record, now)?;
        }
    }
    for (id, original_name, _) in existing
        .iter()
        .filter(|(_, _, position)| *position >= names.len() as i64)
    {
        transaction.execute("UPDATE classes SET name = ?1, updated_at_ms = ?2 WHERE id = ?3", params![original_name, now, id]).map_err(|_| format!("The labels file conflicts with the existing class '{original_name}'. Rename or remove that extra class first."))?;
    }
    if let Some(first) = transaction
        .query_row(
            "SELECT id FROM classes ORDER BY position LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Could not choose the first label: {error}"))?
    {
        transaction.execute("INSERT INTO settings (key, value) VALUES ('active_class_id', ?1) ON CONFLICT(key) DO NOTHING", [first]).map_err(|error| format!("Could not save the active label: {error}"))?;
    }
    touch_project(&transaction, now)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not finish loading labels: {error}"))?;
    state(root_path)
}

pub fn import_yolo(root_path: &str, labels_root: &str) -> Result<ImportReport, String> {
    let directory = Path::new(labels_root);
    if !directory.is_dir() {
        return Err("Choose the folder containing YOLO .txt files.".to_owned());
    }
    let labels_file = ["labels.txt", "classes.txt"]
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file());
    if let Some(path) = labels_file {
        import_labels(root_path, &path.to_string_lossy())?;
    }
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    let classes = classes_from_connection(&connection)?;
    if classes.is_empty() {
        return Err("Load labels.txt or create classes before importing YOLO boxes.".to_owned());
    }
    let images = image_lookup(&connection)?;
    let mut by_relative = HashMap::new();
    let mut by_stem: HashMap<String, Vec<String>> = HashMap::new();
    for (id, relative, file_name, width, height) in &images {
        by_relative.insert(
            with_txt_extension(relative).to_lowercase(),
            (id.clone(), *width, *height),
        );
        let stem = Path::new(file_name)
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or(file_name)
            .to_lowercase();
        by_stem.entry(stem).or_default().push(id.clone());
    }
    let mut imported: HashMap<String, Vec<AnnotationRecord>> = HashMap::new();
    let mut skipped = 0;
    for entry in WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file()
            || path
                .extension()
                .and_then(|v| v.to_str())
                .map(|v| !v.eq_ignore_ascii_case("txt"))
                .unwrap_or(true)
        {
            continue;
        }
        let file_name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
        if file_name.eq_ignore_ascii_case("labels.txt")
            || file_name.eq_ignore_ascii_case("classes.txt")
        {
            continue;
        }
        let relative = path
            .strip_prefix(directory)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        let matched = by_relative.get(&relative).cloned().or_else(|| {
            let stem = path.file_stem()?.to_string_lossy().to_lowercase();
            let ids = by_stem.get(&stem)?;
            if ids.len() != 1 {
                return None;
            }
            images
                .iter()
                .find(|item| item.0 == ids[0])
                .map(|item| (item.0.clone(), item.3, item.4))
        });
        let Some((image_id, width, height)) = matched else {
            skipped += 1;
            continue;
        };
        let text = fs::read_to_string(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let mut rows = Vec::new();
        for (line_index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match parse_yolo_line(trimmed, width as f64, height as f64, &classes, &image_id) {
                Ok(record) => rows.push(record),
                Err(_) => {
                    skipped += 1;
                    eprintln!(
                        "Skipped invalid YOLO row {}:{}",
                        path.display(),
                        line_index + 1
                    );
                }
            }
        }
        imported.insert(image_id, rows);
    }
    let matched_images = imported.len();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for (image_id, rows) in imported {
        before.extend(annotations_for_image(&connection, &image_id)?);
        after.extend(rows);
    }
    let annotations = after.len();
    if !before.is_empty() || !after.is_empty() {
        commit_annotation_change(&mut connection, before, after)?;
    }
    Ok(ImportReport {
        annotations,
        matched_images,
        skipped,
        classes: classes.len(),
        message: format!(
            "Imported {annotations} boxes for {matched_images} images; skipped {skipped} malformed or unmatched rows."
        ),
    })
}

pub fn export_yolo(root_path: &str, destination: &str) -> Result<ImportReport, String> {
    let directory = Path::new(destination);
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create the export folder: {error}"))?;
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    let classes = classes_from_connection(&connection)?;
    if classes.is_empty() {
        return Err("Add at least one class before exporting YOLO labels.".to_owned());
    }
    fs::write(
        directory.join("labels.txt"),
        format!(
            "{}\n",
            classes
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .map_err(|error| format!("Could not write labels.txt: {error}"))?;
    let class_indices: HashMap<_, _> = classes
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.as_str(), index))
        .collect();
    let images = image_lookup(&connection)?;
    let mut annotations = 0;
    let mut annotated_images = 0;
    for (image_id, relative, _, width, height) in &images {
        let rows = annotations_for_image(&connection, image_id)?;
        annotations += rows.len();
        if rows.is_empty() {
            continue;
        }
        annotated_images += 1;
        let mut text = String::new();
        for item in rows {
            let class_index = class_indices
                .get(item.class_id.as_str())
                .ok_or_else(|| "An annotation refers to a missing class.".to_owned())?;
            let g = item.geometry;
            text.push_str(&format!(
                "{} {:.8} {:.8} {:.8} {:.8}\n",
                class_index,
                (g.x + g.width / 2.0) / *width as f64,
                (g.y + g.height / 2.0) / *height as f64,
                g.width / *width as f64,
                g.height / *height as f64
            ));
        }
        let target = directory.join(with_txt_extension(relative));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        fs::write(&target, text)
            .map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    }
    Ok(ImportReport {
        annotations,
        matched_images: annotated_images,
        skipped: 0,
        classes: classes.len(),
        message: format!("Exported {annotations} boxes for {annotated_images} annotated images."),
    })
}

pub fn export_coco(root_path: &str, destination: &str) -> Result<ImportReport, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    let classes = classes_from_connection(&connection)?;
    let images = image_lookup(&connection)?;
    let class_ids: HashMap<_, _> = classes
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.as_str(), index + 1))
        .collect();
    let mut coco_annotations = Vec::new();
    let mut annotation_number = 1;
    for (image_index, (image_id, _, _, _, _)) in images.iter().enumerate() {
        for item in annotations_for_image(&connection, image_id)? {
            let g = item.geometry;
            coco_annotations.push(json!({"id": annotation_number, "image_id": image_index + 1, "category_id": class_ids[item.class_id.as_str()], "bbox": [g.x, g.y, g.width, g.height], "area": g.width * g.height, "iscrowd": 0}));
            annotation_number += 1;
        }
    }
    let document = json!({
        "info": {"description": "Exported by yalt"},
        "images": images.iter().enumerate().map(|(index, (_, path, _, width, height))| json!({"id": index + 1, "file_name": path, "width": width, "height": height})).collect::<Vec<_>>(),
        "categories": classes.iter().enumerate().map(|(index, item)| json!({"id": index + 1, "name": item.name, "supercategory": ""})).collect::<Vec<_>>(),
        "annotations": coco_annotations,
    });
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("Could not encode COCO JSON: {error}"))?;
    fs::write(destination, bytes).map_err(|error| format!("Could not write COCO JSON: {error}"))?;
    Ok(ImportReport {
        annotations: annotation_number - 1,
        matched_images: images.len(),
        skipped: 0,
        classes: classes.len(),
        message: format!("Exported {} COCO boxes.", annotation_number - 1),
    })
}

pub fn import_coco(root_path: &str, source: &str) -> Result<ImportReport, String> {
    let bytes = fs::read(source).map_err(|error| format!("Could not read COCO JSON: {error}"))?;
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("COCO JSON is invalid: {error}"))?;
    let categories = document
        .get("categories")
        .and_then(Value::as_array)
        .ok_or_else(|| "COCO JSON has no categories array.".to_owned())?;
    let coco_images = document
        .get("images")
        .and_then(Value::as_array)
        .ok_or_else(|| "COCO JSON has no images array.".to_owned())?;
    let coco_annotations = document
        .get("annotations")
        .and_then(Value::as_array)
        .ok_or_else(|| "COCO JSON has no annotations array.".to_owned())?;
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_detection(&connection)?;
    let mut classes = classes_from_connection(&connection)?;
    let mut category_map = HashMap::new();
    for category in categories {
        let category_id = integer(category.get("id"), "category id")?;
        let name = category
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "A COCO category has no name.".to_owned())?;
        let class = if let Some(existing) = classes
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(name))
        {
            existing.clone()
        } else {
            let created = create_class(root_path, name)?;
            classes.push(created.clone());
            created
        };
        category_map.insert(category_id, class.id);
    }
    let indexed = image_lookup(&connection)?;
    let mut indexed_paths: HashMap<String, (String, u32, u32)> = HashMap::new();
    let mut indexed_names: HashMap<String, Vec<(String, u32, u32)>> = HashMap::new();
    for (id, relative, name, width, height) in indexed {
        indexed_paths.insert(relative.to_lowercase(), (id.clone(), width, height));
        indexed_names
            .entry(name.to_lowercase())
            .or_default()
            .push((id, width, height));
    }
    let mut coco_image_map = HashMap::new();
    for item in coco_images {
        let id = integer(item.get("id"), "image id")?;
        let name = item
            .get("file_name")
            .and_then(Value::as_str)
            .ok_or_else(|| "A COCO image has no file_name.".to_owned())?
            .replace('\\', "/");
        let found = indexed_paths
            .get(&name.to_lowercase())
            .cloned()
            .or_else(|| {
                let base = Path::new(&name)
                    .file_name()?
                    .to_string_lossy()
                    .to_lowercase();
                let values = indexed_names.get(&base)?;
                (values.len() == 1).then(|| values[0].clone())
            });
        if let Some(value) = found {
            coco_image_map.insert(id, value);
        }
    }
    let now = project::now_ms()?;
    let mut imported: HashMap<String, Vec<AnnotationRecord>> = HashMap::new();
    let mut skipped = 0;
    for item in coco_annotations {
        let parsed = (|| -> Result<AnnotationRecord, String> {
            let image_key = integer(item.get("image_id"), "annotation image_id")?;
            let category_key = integer(item.get("category_id"), "annotation category_id")?;
            let (image_id, width, height) = coco_image_map
                .get(&image_key)
                .ok_or_else(|| "image is not in this project".to_owned())?;
            let class_id = category_map
                .get(&category_key)
                .ok_or_else(|| "category is not defined".to_owned())?;
            let bbox = item
                .get("bbox")
                .and_then(Value::as_array)
                .filter(|v| v.len() >= 4)
                .ok_or_else(|| "bbox must have four numbers".to_owned())?;
            let geometry = BboxGeometry {
                x: number(&bbox[0])?,
                y: number(&bbox[1])?,
                width: number(&bbox[2])?,
                height: number(&bbox[3])?,
            };
            validate_geometry(geometry, *width, *height)?;
            Ok(AnnotationRecord {
                id: Uuid::new_v4().to_string(),
                image_id: image_id.clone(),
                class_id: class_id.clone(),
                kind: "bbox".to_owned(),
                geometry,
                is_visible: true,
                created_at_ms: now,
                updated_at_ms: now,
            })
        })();
        match parsed {
            Ok(record) => imported
                .entry(record.image_id.clone())
                .or_default()
                .push(record),
            Err(_) => skipped += 1,
        }
    }
    let matched_images = imported.len();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for (image_id, rows) in imported {
        before.extend(annotations_for_image(&connection, &image_id)?);
        after.extend(rows);
    }
    let annotations = after.len();
    if !before.is_empty() || !after.is_empty() {
        commit_annotation_change(&mut connection, before, after)?;
    }
    Ok(ImportReport {
        annotations,
        matched_images,
        skipped,
        classes: classes.len(),
        message: format!(
            "Imported {annotations} COCO boxes for {matched_images} images; skipped {skipped}."
        ),
    })
}

pub(crate) fn is_annotation_operation(json: &str) -> bool {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("operation_type")
                .and_then(Value::as_str)
                .map(|kind| kind == "annotation")
        })
        .unwrap_or(false)
}

pub(crate) fn apply_annotation_history(
    connection: &mut Connection,
    sequence: i64,
    json: &str,
    undo: bool,
) -> Result<(), String> {
    let operation: AnnotationOperation = serde_json::from_str(json)
        .map_err(|error| format!("Could not understand annotation history: {error}"))?;
    let desired = if undo {
        &operation.before
    } else {
        &operation.after
    };
    let remove = if undo {
        &operation.after
    } else {
        &operation.before
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not apply annotation history: {error}"))?;
    for item in remove {
        transaction
            .execute("DELETE FROM annotations WHERE id = ?1", [&item.id])
            .map_err(|error| format!("Could not remove an annotation during undo: {error}"))?;
    }
    for item in desired {
        upsert_annotation(&transaction, item)?;
    }
    transaction
        .execute(
            "UPDATE operation_history SET applied = ?1 WHERE sequence = ?2",
            params![!undo, sequence],
        )
        .map_err(|error| format!("Could not update annotation history: {error}"))?;
    touch_project(&transaction, project::now_ms()?)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit annotation history: {error}"))
}

fn commit_annotation_change(
    connection: &mut Connection,
    before: Vec<AnnotationRecord>,
    after: Vec<AnnotationRecord>,
) -> Result<(), String> {
    let operation = AnnotationOperation {
        operation_type: "annotation".to_owned(),
        before: before.clone(),
        after: after.clone(),
    };
    let json = serde_json::to_string(&operation)
        .map_err(|error| format!("Could not encode annotation history: {error}"))?;
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not autosave the annotation: {error}"))?;
    for item in &before {
        transaction
            .execute("DELETE FROM annotations WHERE id = ?1", [&item.id])
            .map_err(|error| format!("Could not replace an annotation: {error}"))?;
    }
    for item in &after {
        upsert_annotation(&transaction, item)?;
    }
    transaction
        .execute("DELETE FROM operation_history WHERE applied = 0", [])
        .map_err(|error| format!("Could not clear obsolete redo history: {error}"))?;
    transaction.execute("INSERT INTO operation_history (operation_json, created_at_ms, applied) VALUES (?1, ?2, 1)", params![json, now]).map_err(|error| format!("Could not record annotation history: {error}"))?;
    touch_project(&transaction, now)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit annotation autosave: {error}"))
}

fn annotations_for_image(
    connection: &Connection,
    image_id: &str,
) -> Result<Vec<AnnotationRecord>, String> {
    let mut statement = connection.prepare("SELECT id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms FROM annotations WHERE image_id = ?1 AND kind = 'bbox' ORDER BY created_at_ms, rowid").map_err(|error| format!("Could not prepare annotations: {error}"))?;
    let rows = statement
        .query_map([image_id], annotation_from_row)
        .map_err(|error| format!("Could not load annotations: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read an annotation: {error}"))
}

fn annotation_by_id(connection: &Connection, id: &str) -> Result<Option<AnnotationRecord>, String> {
    connection.query_row("SELECT id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms FROM annotations WHERE id = ?1 AND kind = 'bbox'", [id], annotation_from_row).optional().map_err(|error| format!("Could not find the box: {error}"))
}

fn annotation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnnotationRecord> {
    let geometry_json: String = row.get(4)?;
    let geometry = serde_json::from_str(&geometry_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(AnnotationRecord {
        id: row.get(0)?,
        image_id: row.get(1)?,
        class_id: row.get(2)?,
        kind: row.get(3)?,
        geometry,
        is_visible: row.get::<_, i64>(5)? != 0,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

fn upsert_annotation(transaction: &Transaction<'_>, item: &AnnotationRecord) -> Result<(), String> {
    let geometry = serde_json::to_string(&item.geometry)
        .map_err(|error| format!("Could not encode box geometry: {error}"))?;
    transaction.execute("INSERT INTO annotations (id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, 'bbox', ?4, ?5, ?6, ?7) ON CONFLICT(id) DO UPDATE SET image_id = excluded.image_id, class_id = excluded.class_id, kind = 'bbox', geometry_json = excluded.geometry_json, is_visible = excluded.is_visible, updated_at_ms = excluded.updated_at_ms", params![item.id, item.image_id, item.class_id, geometry, item.is_visible, item.created_at_ms, item.updated_at_ms]).map(|_| ()).map_err(|error| format!("Could not save a box: {error}"))
}

fn validate_annotation(
    connection: &Connection,
    image_id: &str,
    class_id: &str,
    geometry: BboxGeometry,
) -> Result<(), String> {
    if class_by_id(connection, class_id)?.is_none() {
        return Err("Choose a valid class before drawing a box.".to_owned());
    }
    let dimensions: Option<(u32, u32)> = connection
        .query_row(
            "SELECT width, height FROM images WHERE id = ?1 AND status = 'active'",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Could not find the image: {error}"))?;
    let (width, height) =
        dimensions.ok_or_else(|| "The image is not available for annotation.".to_owned())?;
    validate_geometry(geometry, width, height)
}

fn validate_geometry(g: BboxGeometry, image_width: u32, image_height: u32) -> Result<(), String> {
    if !g.x.is_finite()
        || !g.y.is_finite()
        || !g.width.is_finite()
        || !g.height.is_finite()
        || g.width < 1.0
        || g.height < 1.0
        || g.x < 0.0
        || g.y < 0.0
        || g.x + g.width > image_width as f64 + 0.01
        || g.y + g.height > image_height as f64 + 0.01
    {
        return Err("The box must be at least 1 × 1 px and remain inside the image.".to_owned());
    }
    Ok(())
}

fn parse_yolo_line(
    line: &str,
    image_width: f64,
    image_height: f64,
    classes: &[ClassRecord],
    image_id: &str,
) -> Result<AnnotationRecord, String> {
    let values = line.split_whitespace().collect::<Vec<_>>();
    if values.len() != 5 {
        return Err("YOLO rows need five values.".to_owned());
    }
    let class_index: usize = values[0]
        .parse()
        .map_err(|_| "Invalid YOLO class index.".to_owned())?;
    let numbers = values[1..]
        .iter()
        .map(|value| {
            value
                .parse::<f64>()
                .map_err(|_| "Invalid YOLO coordinate.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if numbers.iter().any(|v| !v.is_finite()) {
        return Err("Invalid YOLO coordinate.".to_owned());
    }
    let width = numbers[2] * image_width;
    let height = numbers[3] * image_height;
    let geometry = BboxGeometry {
        x: numbers[0] * image_width - width / 2.0,
        y: numbers[1] * image_height - height / 2.0,
        width,
        height,
    };
    validate_geometry(geometry, image_width as u32, image_height as u32)?;
    let now = project::now_ms()?;
    Ok(AnnotationRecord {
        id: Uuid::new_v4().to_string(),
        image_id: image_id.to_owned(),
        class_id: classes
            .get(class_index)
            .ok_or_else(|| "YOLO class index is outside labels.txt.".to_owned())?
            .id
            .clone(),
        kind: "bbox".to_owned(),
        geometry,
        is_visible: true,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn classes_from_connection(connection: &Connection) -> Result<Vec<ClassRecord>, String> {
    let mut statement = connection
        .prepare("SELECT id, name, position, color, shortcut FROM classes ORDER BY position")
        .map_err(|error| format!("Could not prepare classes: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(ClassRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
                color: row.get(3)?,
                shortcut: row.get(4)?,
            })
        })
        .map_err(|error| format!("Could not load classes: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read a class: {error}"))
}

fn class_by_id(connection: &Connection, id: &str) -> Result<Option<ClassRecord>, String> {
    connection
        .query_row(
            "SELECT id, name, position, color, shortcut FROM classes WHERE id = ?1",
            [id],
            |row| {
                Ok(ClassRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    position: row.get(2)?,
                    color: row.get(3)?,
                    shortcut: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Could not find the class: {error}"))
}

fn insert_class(
    transaction: &Transaction<'_>,
    record: &ClassRecord,
    now: i64,
) -> Result<(), String> {
    transaction.execute("INSERT INTO classes (id, name, position, color, shortcut, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)", params![record.id, record.name, record.position, record.color, record.shortcut, now]).map(|_| ()).map_err(|error| format!("Could not add class '{}': {error}", record.name))
}

fn compact_class_positions(transaction: &Transaction<'_>) -> Result<(), String> {
    let ids = {
        let mut statement = transaction
            .prepare("SELECT id FROM classes ORDER BY position")
            .map_err(|error| format!("Could not order classes: {error}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Could not order classes: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Could not read classes: {error}"))?
    };
    transaction
        .execute("UPDATE classes SET position = position + 10000", [])
        .map_err(|error| format!("Could not reorder classes: {error}"))?;
    for (position, id) in ids.iter().enumerate() {
        transaction
            .execute(
                "UPDATE classes SET position = ?1, shortcut = ?2 WHERE id = ?3",
                params![position as i64, shortcut_for(position as i64), id],
            )
            .map_err(|error| format!("Could not reorder classes: {error}"))?;
    }
    Ok(())
}

fn image_lookup(connection: &Connection) -> Result<Vec<IndexedImage>, String> {
    let mut statement = connection.prepare("SELECT id, relative_path, file_name, width, height FROM images WHERE status = 'active' ORDER BY sort_order").map_err(|error| format!("Could not inspect project images: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|error| format!("Could not inspect project images: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read project images: {error}"))
}

fn require_detection(connection: &Connection) -> Result<(), String> {
    let task: String = connection
        .query_row("SELECT task_type FROM project LIMIT 1", [], |row| {
            row.get(0)
        })
        .map_err(|error| format!("Could not read the project task: {error}"))?;
    if task != "detection" {
        return Err("This operation is only available in an object detection project.".to_owned());
    }
    Ok(())
}

fn require_spatial_task(connection: &Connection) -> Result<(), String> {
    let task: String = connection
        .query_row("SELECT task_type FROM project LIMIT 1", [], |row| {
            row.get(0)
        })
        .map_err(|error| format!("Could not read the project task: {error}"))?;
    if task != "detection" && task != "segmentation" && task != "classification" {
        return Err("This operation is only available in an annotation project.".to_owned());
    }
    Ok(())
}

fn clear_draft_in_connection(connection: &Connection, image_id: &str) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM drafts WHERE image_id = ?1 AND kind = 'bbox'",
            [image_id],
        )
        .map(|_| ())
        .map_err(|error| format!("Could not clear the box draft: {error}"))
}

fn valid_class_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 100 || trimmed.contains(['\n', '\r']) {
        return Err("Class names must contain 1–100 characters on one line.".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn label_names(text: &str) -> Result<Vec<String>, String> {
    let names = text
        .lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}'))
        .filter(|line| !line.is_empty())
        .map(valid_class_name)
        .collect::<Result<Vec<_>, _>>()?;
    if names.is_empty() {
        return Err("The labels file is empty.".to_owned());
    }
    let mut seen = std::collections::HashSet::new();
    if names.iter().any(|name| !seen.insert(name.to_lowercase())) {
        return Err("The labels file contains duplicate class names.".to_owned());
    }
    Ok(names)
}

fn shortcut_for(position: i64) -> Option<String> {
    match position {
        0..=8 => Some((position + 1).to_string()),
        9 => Some("0".to_owned()),
        _ => None,
    }
}

fn spectrum_color(position: usize) -> String {
    const COLORS: [&str; 12] = [
        "#9B6CFF", "#657BFF", "#3E9DEB", "#35C9D0", "#43CE87", "#8FCB55", "#D4C84E", "#E8A245",
        "#ED7354", "#E65362", "#D54D9A", "#A95AC8",
    ];
    COLORS[position % COLORS.len()].to_owned()
}

fn touch_project(transaction: &Transaction<'_>, now: i64) -> Result<(), String> {
    transaction
        .execute("UPDATE project SET updated_at_ms = ?1", [now])
        .map(|_| ())
        .map_err(|error| format!("Could not update project state: {error}"))
}

fn with_txt_extension(relative: &str) -> String {
    let mut path = PathBuf::from(relative);
    path.set_extension("txt");
    path.to_string_lossy().replace('\\', "/")
}

fn integer(value: Option<&Value>, label: &str) -> Result<i64, String> {
    value
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("COCO {label} must be an integer."))
}
fn number(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| "COCO bbox values must be finite numbers.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    fn project_with_image() -> (TempDir, String, String) {
        let directory = TempDir::new().expect("temp");
        ImageBuffer::from_pixel(200, 100, Rgb([1_u8, 2_u8, 3_u8]))
            .save(directory.path().join("frame.jpg"))
            .expect("image");
        let root = directory.path().to_string_lossy().into_owned();
        project::create(&root, "Detection", "detection", None).expect("project");
        let image_id = project::list_images(&root, 10, 0).expect("images")[0]
            .id
            .clone();
        (directory, root, image_id)
    }

    #[test]
    fn annotations_round_trip_and_undo() {
        let (_directory, root, image_id) = project_with_image();
        let class = create_class(&root, "person").expect("class");
        let geometry = BboxGeometry {
            x: 20.0,
            y: 10.0,
            width: 60.0,
            height: 40.0,
        };
        let annotation =
            create_annotation(&root, &image_id, &class.id, geometry).expect("annotation");
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0].geometry,
            geometry
        );
        assert_eq!(
            crate::workspace::undo(&root).expect("undo"),
            Some(image_id.clone())
        );
        assert!(list_annotations(&root, &image_id).expect("list").is_empty());
        assert_eq!(
            crate::workspace::redo(&root).expect("redo"),
            Some(image_id.clone())
        );
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0].id,
            annotation.id
        );
    }

    #[test]
    fn yolo_export_import_preserves_pixel_geometry() {
        let (directory, root, image_id) = project_with_image();
        let class = create_class(&root, "vehicle").expect("class");
        let expected = BboxGeometry {
            x: 24.0,
            y: 12.0,
            width: 80.0,
            height: 50.0,
        };
        create_annotation(&root, &image_id, &class.id, expected).expect("annotation");
        let export = directory.path().join("yolo");
        export_yolo(&root, &export.to_string_lossy()).expect("export");
        delete_annotation(
            &root,
            &list_annotations(&root, &image_id).expect("list")[0].id,
        )
        .expect("delete");
        import_yolo(&root, &export.to_string_lossy()).expect("import");
        let actual = list_annotations(&root, &image_id).expect("list")[0].geometry;
        assert!((actual.x - expected.x).abs() < 0.0001);
        assert!((actual.y - expected.y).abs() < 0.0001);
        assert!((actual.width - expected.width).abs() < 0.0001);
        assert!((actual.height - expected.height).abs() < 0.0001);
    }

    #[test]
    fn yolo_export_omits_empty_image_label_files() {
        let directory = TempDir::new().expect("temp");
        for name in ["frame.jpg", "without-box.jpg"] {
            ImageBuffer::from_pixel(200, 100, Rgb([1_u8, 2_u8, 3_u8]))
                .save(directory.path().join(name))
                .expect("image");
        }
        let root = directory.path().to_string_lossy().into_owned();
        project::create(&root, "Detection", "detection", None).expect("project");
        let image_id = project::list_images(&root, 10, 0)
            .expect("images")
            .into_iter()
            .find(|image| image.file_name == "frame.jpg")
            .expect("frame")
            .id;
        let class = create_class(&root, "vehicle").expect("class");
        create_annotation(
            &root,
            &image_id,
            &class.id,
            BboxGeometry {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 20.0,
            },
        )
        .expect("box");
        let destination = directory.path().join("labels");
        let report = export_yolo(&root, &destination.to_string_lossy()).expect("export");
        assert!(destination.join("frame.txt").is_file());
        assert!(!destination.join("without-box.txt").exists());
        assert_eq!(report.matched_images, 1);
    }

    #[test]
    fn coco_export_import_preserves_pixel_geometry() {
        let (directory, root, image_id) = project_with_image();
        let class = create_class(&root, "person").expect("class");
        let expected = BboxGeometry {
            x: 4.5,
            y: 8.25,
            width: 31.5,
            height: 44.0,
        };
        create_annotation(&root, &image_id, &class.id, expected).expect("annotation");
        let path = directory.path().join("annotations.json");
        export_coco(&root, &path.to_string_lossy()).expect("export");
        delete_annotation(
            &root,
            &list_annotations(&root, &image_id).expect("list")[0].id,
        )
        .expect("delete");
        import_coco(&root, &path.to_string_lossy()).expect("import");
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0].geometry,
            expected
        );
    }

    #[test]
    fn imports_golden_yolo_fixture() {
        let (_directory, root, image_id) = project_with_image();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/yolo");
        let report = import_yolo(&root, &fixture.to_string_lossy()).expect("golden import");
        assert_eq!(report.annotations, 1);
        assert_eq!(
            list_annotations(&root, &image_id).expect("boxes")[0].geometry,
            BboxGeometry {
                x: 24.0,
                y: 12.0,
                width: 80.0,
                height: 50.0
            }
        );
    }

    #[test]
    fn imports_golden_coco_fixture() {
        let (_directory, root, image_id) = project_with_image();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/coco_bbox.json");
        let report = import_coco(&root, &fixture.to_string_lossy()).expect("golden import");
        assert_eq!(report.annotations, 1);
        assert_eq!(
            list_annotations(&root, &image_id).expect("boxes")[0].geometry,
            BboxGeometry {
                x: 24.0,
                y: 12.0,
                width: 80.0,
                height: 50.0
            }
        );
    }

    #[test]
    fn draft_and_active_class_survive_reopen() {
        let (_directory, root, image_id) = project_with_image();
        let class = create_class(&root, "animal").expect("class");
        set_active_class(&root, &class.id).expect("active");
        let draft = BboxDraft {
            image_id: image_id.clone(),
            class_id: class.id.clone(),
            x: 15.0,
            y: 18.0,
        };
        save_draft(&root, Some(draft.clone()), &image_id).expect("draft");
        project::open(&root).expect("reopen");
        assert_eq!(state(&root).expect("state").active_class_id, Some(class.id));
        assert_eq!(load_draft(&root, &image_id).expect("load"), Some(draft));
    }

    #[test]
    fn malformed_yolo_rows_are_counted_in_the_import_report() {
        let (directory, root, image_id) = project_with_image();
        create_class(&root, "vehicle").expect("class");
        let labels = directory.path().join("incoming-labels");
        fs::create_dir(&labels).expect("labels folder");
        fs::write(labels.join("frame.txt"), "0 0.5 0.5 nope 0.2\n").expect("label");

        let report = import_yolo(&root, &labels.to_string_lossy()).expect("import report");
        assert_eq!(report.skipped, 1);
        assert!(report.message.contains("skipped 1"));
        assert!(list_annotations(&root, &image_id)
            .expect("annotations")
            .is_empty());
    }
}
