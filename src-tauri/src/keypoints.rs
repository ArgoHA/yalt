use crate::detection::{self, BboxGeometry, ClassRecord, ImportReport};
use crate::project;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use uuid::Uuid;

/// COCO `area` for objects without masks: the OKS scale D-FINE-seg uses for
/// YOLO pose labels (`0.53 * box area`).
const BOX_AREA_FACTOR: f64 = 0.53;
const SCHEMA_KEY: &str = "keypoint_schema";
const MAX_KEYPOINTS: usize = 256;

/// The ordered points every object in the project carries (D-FINE-seg's K).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeypointSchema {
    pub names: Vec<String>,
    /// Zero-based point pairs, drawing only.
    #[serde(default)]
    pub skeleton: Vec<[usize; 2]>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Keypoint {
    pub x: f64,
    pub y: f64,
    /// 0 unlabeled, 1 labeled but occluded, 2 visible (the COCO / YOLO-pose flags).
    pub v: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeypointGeometry {
    pub bbox: BboxGeometry,
    pub keypoints: Vec<Keypoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeypointAnnotationRecord {
    pub id: String,
    pub image_id: String,
    pub class_id: String,
    pub kind: String,
    pub geometry: KeypointGeometry,
    pub is_visible: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeypointOperation {
    operation_type: String,
    before: Vec<KeypointAnnotationRecord>,
    after: Vec<KeypointAnnotationRecord>,
}

pub fn schema(root_path: &str) -> Result<KeypointSchema, String> {
    let connection = open(root_path)?;
    load_schema(&connection)
}

pub fn set_schema(root_path: &str, schema: KeypointSchema) -> Result<KeypointSchema, String> {
    let schema = validate_schema(schema)?;
    let connection = open(root_path)?;
    let current = load_schema(&connection)?;
    if schema.names.len() < current.names.len() && has_annotations(&connection)? {
        return Err(
            "Keypoints can be renamed or added once objects exist. Delete every object before removing points."
                .to_owned(),
        );
    }
    save_schema(&connection, &schema)?;
    Ok(schema)
}

pub fn validate_schema(schema: KeypointSchema) -> Result<KeypointSchema, String> {
    let names = schema
        .names
        .iter()
        .map(|name| {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.len() > 100 || trimmed.contains(['\n', '\r', ',']) {
                Err(
                    "Keypoint names must contain 1–100 characters without commas or line breaks."
                        .to_owned(),
                )
            } else {
                Ok(trimmed.to_owned())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if names.len() > MAX_KEYPOINTS {
        return Err(format!(
            "A project supports up to {MAX_KEYPOINTS} keypoints."
        ));
    }
    let mut seen = HashSet::new();
    if names.iter().any(|name| !seen.insert(name.to_lowercase())) {
        return Err("Keypoint names must be unique.".to_owned());
    }
    // Pairs that no longer fit the point list are dropped instead of rejected:
    // the skeleton only affects drawing and COCO metadata.
    let mut skeleton: Vec<[usize; 2]> = Vec::new();
    for [a, b] in schema.skeleton {
        if a < names.len()
            && b < names.len()
            && a != b
            && !skeleton.contains(&[a, b])
            && !skeleton.contains(&[b, a])
        {
            skeleton.push([a, b]);
        }
    }
    Ok(KeypointSchema { names, skeleton })
}

pub fn list_annotations(
    root_path: &str,
    image_id: &str,
) -> Result<Vec<KeypointAnnotationRecord>, String> {
    let connection = open(root_path)?;
    let count = load_schema(&connection)?.names.len();
    annotations_for_image(&connection, image_id, count)
}

pub fn create_annotation(
    root_path: &str,
    image_id: &str,
    class_id: &str,
    geometry: KeypointGeometry,
) -> Result<KeypointAnnotationRecord, String> {
    let mut connection = open(root_path)?;
    let count = load_schema(&connection)?.names.len();
    let geometry = validate_annotation(&connection, image_id, class_id, geometry, count)?;
    let now = project::now_ms()?;
    let record = KeypointAnnotationRecord {
        id: Uuid::new_v4().to_string(),
        image_id: image_id.to_owned(),
        class_id: class_id.to_owned(),
        kind: "keypoints".to_owned(),
        geometry,
        is_visible: true,
        created_at_ms: now,
        updated_at_ms: now,
    };
    commit_change(&mut connection, vec![], vec![record.clone()])?;
    Ok(record)
}

pub fn update_annotation(
    root_path: &str,
    annotation_id: &str,
    class_id: &str,
    geometry: KeypointGeometry,
    is_visible: bool,
) -> Result<KeypointAnnotationRecord, String> {
    let mut connection = open(root_path)?;
    let count = load_schema(&connection)?.names.len();
    let before = annotation_by_id(&connection, annotation_id, count)?
        .ok_or_else(|| "The object no longer exists.".to_owned())?;
    let geometry = validate_annotation(&connection, &before.image_id, class_id, geometry, count)?;
    let mut after = before.clone();
    after.class_id = class_id.to_owned();
    after.geometry = geometry;
    after.is_visible = is_visible;
    after.updated_at_ms = project::now_ms()?;
    commit_change(&mut connection, vec![before], vec![after.clone()])?;
    Ok(after)
}

pub fn delete_annotation(root_path: &str, annotation_id: &str) -> Result<(), String> {
    let mut connection = open(root_path)?;
    let count = load_schema(&connection)?.names.len();
    let before = annotation_by_id(&connection, annotation_id, count)?
        .ok_or_else(|| "The object no longer exists.".to_owned())?;
    commit_change(&mut connection, vec![before], vec![])
}

/// Writes YOLO-pose rows: `class xc yc w h x1 y1 v1 ... xK yK vK`, normalized,
/// with `0 0 0` for unlabeled points (the D-FINE-seg / Ultralytics `[K, 3]` layout).
pub fn export_yolo(root_path: &str, destination: &str) -> Result<ImportReport, String> {
    let directory = Path::new(destination);
    let connection = open(root_path)?;
    let classes = detection::classes_from_connection(&connection)?;
    if classes.is_empty() {
        return Err("Add at least one class before exporting YOLO keypoints.".to_owned());
    }
    let count = load_schema(&connection)?.names.len();
    let images = detection::image_lookup(&connection)?;
    crate::yolo::prepare_export(directory, images.iter().map(|image| image.1.as_str()))?;
    crate::yolo::write_class_list(directory, classes.iter().map(|item| item.name.as_str()))?;
    let class_indices = class_indices(&classes);
    let mut objects = 0;
    let mut annotated_images = 0;
    for (image_id, relative, _, width, height) in &images {
        let rows = annotations_for_image(&connection, image_id, count)?;
        if rows.is_empty() {
            continue;
        }
        annotated_images += 1;
        objects += rows.len();
        let (width, height) = (*width as f64, *height as f64);
        let mut text = String::new();
        for item in rows {
            let class_index = class_indices
                .get(item.class_id.as_str())
                .ok_or_else(|| "An object refers to a missing class.".to_owned())?;
            let b = item.geometry.bbox;
            text.push_str(&format!(
                "{} {:.8} {:.8} {:.8} {:.8}",
                class_index,
                (b.x + b.width / 2.0) / width,
                (b.y + b.height / 2.0) / height,
                b.width / width,
                b.height / height
            ));
            for point in item.geometry.keypoints {
                if point.v == 0 {
                    text.push_str(" 0 0 0");
                } else {
                    text.push_str(&format!(
                        " {:.8} {:.8} {}",
                        point.x / width,
                        point.y / height,
                        point.v
                    ));
                }
            }
            text.push('\n');
        }
        let target = directory.join(detection::with_txt_extension(relative));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        fs::write(&target, text)
            .map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    }
    Ok(ImportReport {
        annotations: objects,
        matched_images: annotated_images,
        skipped: 0,
        classes: classes.len(),
        message: format!(
            "Exported {objects} objects with {count} keypoints each (kpt_shape [{count}, 3]) for {annotated_images} annotated images."
        ),
    })
}

/// Reads YOLO-pose rows with `[K, 3]` or flag-less `[K, 2]` points. A project
/// without keypoint names takes K from the first row.
pub fn import_yolo(root_path: &str, labels_root: &str) -> Result<ImportReport, String> {
    let directory = Path::new(labels_root);
    if !directory.is_dir() {
        return Err("Choose the folder containing YOLO keypoint .txt files.".to_owned());
    }
    if let Some(path) = ["labels.txt", "classes.txt"]
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file())
    {
        detection::import_labels(root_path, &path.to_string_lossy())?;
    }
    let mut connection = open(root_path)?;
    let classes = detection::classes_from_connection(&connection)?;
    if classes.is_empty() {
        return Err(
            "Load labels.txt or create classes before importing YOLO keypoints.".to_owned(),
        );
    }
    let images = detection::image_lookup(&connection)?;
    let (files, mut skipped) = crate::yolo::match_label_files(directory, &images);
    let mut sources = Vec::with_capacity(files.len());
    for (path, image_id, width, height) in files {
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        sources.push((image_id, width, height, text));
    }
    let mut schema = load_schema(&connection)?;
    if schema.names.is_empty() {
        let first_row = sources
            .iter()
            .flat_map(|source| source.3.lines())
            .map(str::trim)
            .find(|line| !line.is_empty());
        let count = first_row.map_or(0, |row| {
            inferred_point_count(row.split_whitespace().count())
        });
        if count > 0 {
            schema.names = generated_names(count);
            save_schema(&connection, &schema)?;
        }
    }
    let count = schema.names.len();
    let now = project::now_ms()?;
    let mut imported: HashMap<String, Vec<KeypointAnnotationRecord>> = HashMap::new();
    for (image_id, width, height, text) in sources {
        let mut rows = Vec::new();
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            match parse_yolo_row(line, count, width, height, &classes, &image_id, now) {
                Ok(record) => rows.push(record),
                Err(_) => skipped += 1,
            }
        }
        imported.insert(image_id, rows);
    }
    let (annotations, matched_images) =
        replace_image_annotations(&mut connection, imported, count)?;
    Ok(ImportReport {
        annotations,
        matched_images,
        skipped,
        classes: classes.len(),
        message: format!(
            "Imported {annotations} keypoint objects ({count} points each) for {matched_images} images; skipped {skipped} malformed or unmatched rows."
        ),
    })
}

pub fn export_coco(root_path: &str, destination: &str) -> Result<ImportReport, String> {
    let connection = open(root_path)?;
    let classes = detection::classes_from_connection(&connection)?;
    let schema = load_schema(&connection)?;
    let count = schema.names.len();
    let images = detection::image_lookup(&connection)?;
    let category_ids: HashMap<_, _> = classes
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.as_str(), index + 1))
        .collect();
    let skeleton: Vec<[usize; 2]> = schema
        .skeleton
        .iter()
        .map(|[a, b]| [a + 1, b + 1])
        .collect();
    let mut output = Vec::new();
    for (image_index, (image_id, _, _, _, _)) in images.iter().enumerate() {
        for item in annotations_for_image(&connection, image_id, count)? {
            let b = item.geometry.bbox;
            let keypoints: Vec<Value> = item
                .geometry
                .keypoints
                .iter()
                .flat_map(|point| [json!(point.x), json!(point.y), json!(point.v)])
                .collect();
            let labeled = item
                .geometry
                .keypoints
                .iter()
                .filter(|point| point.v > 0)
                .count();
            output.push(json!({
                "id": output.len() + 1,
                "image_id": image_index + 1,
                "category_id": category_ids[item.class_id.as_str()],
                "bbox": [b.x, b.y, b.width, b.height],
                "area": b.width * b.height * BOX_AREA_FACTOR,
                "iscrowd": 0,
                "keypoints": keypoints,
                "num_keypoints": labeled,
            }));
        }
    }
    let exported = output.len();
    let document = json!({
        "info": {"description": "Exported by yalt"},
        "images": images.iter().enumerate().map(|(index, (_, path, _, width, height))| json!({"id": index + 1, "file_name": path, "width": width, "height": height})).collect::<Vec<_>>(),
        "categories": classes.iter().enumerate().map(|(index, item)| json!({"id": index + 1, "name": item.name, "supercategory": "", "keypoints": schema.names, "skeleton": skeleton})).collect::<Vec<_>>(),
        "annotations": output,
    });
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("Could not encode COCO JSON: {error}"))?;
    fs::write(destination, bytes).map_err(|error| format!("Could not write COCO JSON: {error}"))?;
    Ok(ImportReport {
        annotations: exported,
        matched_images: images.len(),
        skipped: 0,
        classes: classes.len(),
        message: format!("Exported {exported} COCO keypoint objects with {count} points each."),
    })
}

/// Reads COCO keypoint annotations. A project without keypoint names adopts the
/// category's `keypoints` and `skeleton`; otherwise the point count must match.
pub fn import_coco(root_path: &str, source: &str) -> Result<ImportReport, String> {
    let bytes = fs::read(source).map_err(|error| format!("Could not read COCO JSON: {error}"))?;
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("COCO JSON is invalid: {error}"))?;
    let categories = array(&document, "categories")?;
    let coco_images = array(&document, "images")?;
    let coco_annotations = array(&document, "annotations")?;
    let mut connection = open(root_path)?;

    let mut classes = detection::classes_from_connection(&connection)?;
    let mut category_map = HashMap::new();
    let mut coco_schema: Option<KeypointSchema> = None;
    for category in categories {
        let category_id = integer(category.get("id"), "category id")?;
        let name = category
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "A COCO category has no name.".to_owned())?;
        let class = match classes
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(name))
        {
            Some(existing) => existing.clone(),
            None => {
                let created = detection::create_class(root_path, name)?;
                classes.push(created.clone());
                created
            }
        };
        category_map.insert(category_id, class.id);
        if coco_schema.is_none() {
            coco_schema = category_schema(category)?;
        }
    }
    let coco_count = coco_schema.as_ref().map_or_else(
        || {
            coco_annotations
                .iter()
                .filter_map(|item| Some(item.get("keypoints")?.as_array()?.len() / 3))
                .max()
                .unwrap_or(0)
        },
        |schema| schema.names.len(),
    );
    let mut schema = load_schema(&connection)?;
    if schema.names.is_empty() && coco_count > 0 {
        schema = validate_schema(coco_schema.unwrap_or_else(|| KeypointSchema {
            names: generated_names(coco_count),
            skeleton: Vec::new(),
        }))?;
        save_schema(&connection, &schema)?;
    } else if coco_count > 0 && coco_count != schema.names.len() {
        return Err(format!(
            "This COCO file has {coco_count} keypoints per object, but the project defines {}.",
            schema.names.len()
        ));
    } else if let Some(skeleton) = coco_schema
        .filter(|coco| schema.skeleton.is_empty() && !coco.skeleton.is_empty())
        .map(|coco| coco.skeleton)
    {
        schema = validate_schema(KeypointSchema {
            names: schema.names,
            skeleton,
        })?;
        save_schema(&connection, &schema)?;
    }
    let count = schema.names.len();

    let image_map = coco_image_map(&connection, coco_images)?;
    let now = project::now_ms()?;
    let mut imported: HashMap<String, Vec<KeypointAnnotationRecord>> = HashMap::new();
    let mut skipped = 0;
    for item in coco_annotations {
        let parsed = (|| -> Result<KeypointAnnotationRecord, String> {
            if item.get("iscrowd").and_then(Value::as_i64).unwrap_or(0) != 0 {
                return Err("crowd regions are not objects".to_owned());
            }
            let image_key = integer(item.get("image_id"), "annotation image_id")?;
            let category_key = integer(item.get("category_id"), "annotation category_id")?;
            let (image_id, width, height) = image_map
                .get(&image_key)
                .ok_or_else(|| "image is not in this project".to_owned())?;
            let class_id = category_map
                .get(&category_key)
                .ok_or_else(|| "category is not defined".to_owned())?;
            let bbox = item
                .get("bbox")
                .and_then(Value::as_array)
                .filter(|values| values.len() >= 4)
                .ok_or_else(|| "bbox must have four numbers".to_owned())?;
            let bbox = BboxGeometry {
                x: number(&bbox[0])?,
                y: number(&bbox[1])?,
                width: number(&bbox[2])?,
                height: number(&bbox[3])?,
            };
            detection::validate_geometry(bbox, *width, *height)?;
            let values = item
                .get("keypoints")
                .and_then(Value::as_array)
                .filter(|values| !values.is_empty());
            let keypoints = match values {
                // Classes without points carry no `keypoints`: all slots stay unlabeled.
                None => vec![Keypoint::default(); count],
                Some(values) if values.len() == count * 3 => values
                    .chunks_exact(3)
                    .map(|triple| {
                        let flag = number(&triple[2])?;
                        if flag != 0.0 && flag != 1.0 && flag != 2.0 {
                            return Err("keypoint flags must be 0, 1, or 2".to_owned());
                        }
                        Ok(clean_point(
                            number(&triple[0])?,
                            number(&triple[1])?,
                            flag as u8,
                            *width,
                            *height,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                Some(_) => return Err("keypoint count does not match the project".to_owned()),
            };
            Ok(KeypointAnnotationRecord {
                id: Uuid::new_v4().to_string(),
                image_id: image_id.clone(),
                class_id: class_id.clone(),
                kind: "keypoints".to_owned(),
                geometry: KeypointGeometry { bbox, keypoints },
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
    let (annotations, matched_images) =
        replace_image_annotations(&mut connection, imported, count)?;
    Ok(ImportReport {
        annotations,
        matched_images,
        skipped,
        classes: classes.len(),
        message: format!(
            "Imported {annotations} COCO keypoint objects ({count} points each) for {matched_images} images; skipped {skipped}."
        ),
    })
}

pub(crate) fn is_keypoint_operation(json: &str) -> bool {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("operation_type")
                .and_then(Value::as_str)
                .map(|kind| kind == "keypoint_annotation")
        })
        .unwrap_or(false)
}

pub(crate) fn apply_keypoint_history(
    connection: &mut Connection,
    sequence: i64,
    json: &str,
    undo: bool,
) -> Result<(), String> {
    let operation: KeypointOperation = serde_json::from_str(json)
        .map_err(|error| format!("Could not understand keypoint history: {error}"))?;
    let (desired, remove) = if undo {
        (&operation.before, &operation.after)
    } else {
        (&operation.after, &operation.before)
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not apply keypoint history: {error}"))?;
    for item in remove {
        transaction
            .execute("DELETE FROM annotations WHERE id = ?1", [&item.id])
            .map_err(|error| format!("Could not remove an object during undo: {error}"))?;
    }
    for item in desired {
        upsert_annotation(&transaction, item)?;
    }
    transaction
        .execute(
            "UPDATE operation_history SET applied = ?1 WHERE sequence = ?2",
            params![!undo, sequence],
        )
        .map_err(|error| format!("Could not update keypoint history: {error}"))?;
    detection::touch_project(&transaction, project::now_ms()?)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit keypoint history: {error}"))
}

fn commit_change(
    connection: &mut Connection,
    before: Vec<KeypointAnnotationRecord>,
    after: Vec<KeypointAnnotationRecord>,
) -> Result<(), String> {
    let operation = KeypointOperation {
        operation_type: "keypoint_annotation".to_owned(),
        before,
        after,
    };
    let json = serde_json::to_string(&operation)
        .map_err(|error| format!("Could not encode keypoint history: {error}"))?;
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not autosave the object: {error}"))?;
    for item in &operation.before {
        transaction
            .execute("DELETE FROM annotations WHERE id = ?1", [&item.id])
            .map_err(|error| format!("Could not replace an object: {error}"))?;
    }
    for item in &operation.after {
        upsert_annotation(&transaction, item)?;
    }
    transaction
        .execute("DELETE FROM operation_history WHERE applied = 0", [])
        .map_err(|error| format!("Could not clear obsolete redo history: {error}"))?;
    transaction
        .execute(
            "INSERT INTO operation_history (operation_json, created_at_ms, applied) VALUES (?1, ?2, 1)",
            params![json, now],
        )
        .map_err(|error| format!("Could not record keypoint history: {error}"))?;
    detection::touch_project(&transaction, now)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit keypoint autosave: {error}"))
}

/// Replaces the keypoint objects of every image that appears in `imported`
/// as one undoable step. Returns (objects imported, images touched).
fn replace_image_annotations(
    connection: &mut Connection,
    imported: HashMap<String, Vec<KeypointAnnotationRecord>>,
    count: usize,
) -> Result<(usize, usize), String> {
    let matched_images = imported.len();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for (image_id, rows) in imported {
        before.extend(annotations_for_image(connection, &image_id, count)?);
        after.extend(rows);
    }
    let annotations = after.len();
    if !before.is_empty() || !after.is_empty() {
        commit_change(connection, before, after)?;
    }
    Ok((annotations, matched_images))
}

fn annotations_for_image(
    connection: &Connection,
    image_id: &str,
    count: usize,
) -> Result<Vec<KeypointAnnotationRecord>, String> {
    let mut statement = connection
        .prepare("SELECT id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms FROM annotations WHERE image_id = ?1 AND kind = 'keypoints' ORDER BY created_at_ms, rowid")
        .map_err(|error| format!("Could not prepare keypoint objects: {error}"))?;
    let rows = statement
        .query_map([image_id], annotation_from_row)
        .map_err(|error| format!("Could not load keypoint objects: {error}"))?;
    rows.map(|row| row.map(|record| normalized(record, count)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read a keypoint object: {error}"))
}

fn annotation_by_id(
    connection: &Connection,
    id: &str,
    count: usize,
) -> Result<Option<KeypointAnnotationRecord>, String> {
    connection
        .query_row("SELECT id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms FROM annotations WHERE id = ?1 AND kind = 'keypoints'", [id], annotation_from_row)
        .optional()
        .map(|record| record.map(|record| normalized(record, count)))
        .map_err(|error| format!("Could not find the object: {error}"))
}

/// Stored objects keep the slot count of the schema they were saved under;
/// points added to the schema later read as unlabeled.
fn normalized(mut record: KeypointAnnotationRecord, count: usize) -> KeypointAnnotationRecord {
    record.geometry.keypoints.resize(count, Keypoint::default());
    record
}

fn annotation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<KeypointAnnotationRecord> {
    let json: String = row.get(4)?;
    let geometry = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(KeypointAnnotationRecord {
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

fn upsert_annotation(
    transaction: &Transaction<'_>,
    item: &KeypointAnnotationRecord,
) -> Result<(), String> {
    let geometry = serde_json::to_string(&item.geometry)
        .map_err(|error| format!("Could not encode keypoints: {error}"))?;
    transaction
        .execute("INSERT INTO annotations (id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, 'keypoints', ?4, ?5, ?6, ?7) ON CONFLICT(id) DO UPDATE SET image_id = excluded.image_id, class_id = excluded.class_id, kind = 'keypoints', geometry_json = excluded.geometry_json, is_visible = excluded.is_visible, updated_at_ms = excluded.updated_at_ms", params![item.id, item.image_id, item.class_id, geometry, item.is_visible, item.created_at_ms, item.updated_at_ms])
        .map(|_| ())
        .map_err(|error| format!("Could not save an object: {error}"))
}

fn validate_annotation(
    connection: &Connection,
    image_id: &str,
    class_id: &str,
    mut geometry: KeypointGeometry,
    count: usize,
) -> Result<KeypointGeometry, String> {
    if detection::class_by_id(connection, class_id)?.is_none() {
        return Err("Choose a valid class before drawing an object.".to_owned());
    }
    let (width, height): (u32, u32) = connection
        .query_row(
            "SELECT width, height FROM images WHERE id = ?1 AND status = 'active'",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Could not find the image: {error}"))?
        .ok_or_else(|| "The image is not available for annotation.".to_owned())?;
    detection::validate_geometry(geometry.bbox, width, height)?;
    if geometry.keypoints.len() > count {
        return Err(format!(
            "Each object has {count} keypoints in this project."
        ));
    }
    geometry.keypoints.resize(count, Keypoint::default());
    for point in &mut geometry.keypoints {
        if point.v == 0 {
            *point = Keypoint::default();
        } else if point.v > 2
            || !point.x.is_finite()
            || !point.y.is_finite()
            || point.x < 0.0
            || point.y < 0.0
            || point.x > width as f64
            || point.y > height as f64
        {
            return Err(
                "Keypoints must remain inside the image with a 0, 1, or 2 flag.".to_owned(),
            );
        }
    }
    Ok(geometry)
}

fn parse_yolo_row(
    line: &str,
    count: usize,
    width: u32,
    height: u32,
    classes: &[ClassRecord],
    image_id: &str,
    now: i64,
) -> Result<KeypointAnnotationRecord, String> {
    let values = line.split_whitespace().collect::<Vec<_>>();
    let dims = if values.len() == 5 + 3 * count {
        3
    } else if count > 0 && values.len() == 5 + 2 * count {
        2
    } else {
        return Err(format!("A YOLO keypoint row needs 5 + {count} × 3 values."));
    };
    let class = values[0]
        .parse::<usize>()
        .ok()
        .and_then(|index| classes.get(index))
        .ok_or_else(|| "The YOLO class index is outside labels.txt.".to_owned())?;
    let numbers = values[1..]
        .iter()
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or_else(|| "Invalid YOLO value.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (w, h) = (width as f64, height as f64);
    let box_width = numbers[2] * w;
    let box_height = numbers[3] * h;
    let bbox = BboxGeometry {
        x: numbers[0] * w - box_width / 2.0,
        y: numbers[1] * h - box_height / 2.0,
        width: box_width,
        height: box_height,
    };
    detection::validate_geometry(bbox, width, height)?;
    let keypoints = numbers[4..]
        .chunks_exact(dims)
        .map(|point| {
            // `[K, 2]` labels have no flags: (0, 0) means unlabeled, anything else is visible.
            let flag = match dims {
                3 => point[2],
                _ if point[0] == 0.0 && point[1] == 0.0 => 0.0,
                _ => 2.0,
            };
            if flag != 0.0 && flag != 1.0 && flag != 2.0 {
                return Err("Keypoint flags must be 0, 1, or 2.".to_owned());
            }
            Ok(clean_point(
                point[0] * w,
                point[1] * h,
                flag as u8,
                width,
                height,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(KeypointAnnotationRecord {
        id: Uuid::new_v4().to_string(),
        image_id: image_id.to_owned(),
        class_id: class.id.clone(),
        kind: "keypoints".to_owned(),
        geometry: KeypointGeometry { bbox, keypoints },
        is_visible: true,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// YOLO-pose rows are `class + box(4) + K × 3`; `K × 2` (no flags) is the fallback.
fn inferred_point_count(values: usize) -> usize {
    match values.checked_sub(5) {
        Some(rest) if rest > 0 && rest % 3 == 0 => rest / 3,
        Some(rest) if rest > 0 && rest % 2 == 0 => rest / 2,
        _ => 0,
    }
}

/// Exchange files may round a point just past the border; a point further out
/// is not on the image and becomes unlabeled, as in D-FINE-seg.
fn clean_point(x: f64, y: f64, v: u8, width: u32, height: u32) -> Keypoint {
    let (w, h) = (width as f64, height as f64);
    if v == 0 || x < -1.0 || y < -1.0 || x > w + 1.0 || y > h + 1.0 {
        return Keypoint::default();
    }
    Keypoint {
        x: x.clamp(0.0, w),
        y: y.clamp(0.0, h),
        v,
    }
}

fn category_schema(category: &Value) -> Result<Option<KeypointSchema>, String> {
    let Some(names) = category
        .get("keypoints")
        .and_then(Value::as_array)
        .filter(|names| !names.is_empty())
    else {
        return Ok(None);
    };
    let names = names
        .iter()
        .map(|name| name.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "COCO keypoint names must be strings.".to_owned())?;
    // COCO stores skeleton pairs one-based.
    let skeleton = category
        .get("skeleton")
        .and_then(Value::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| {
                    let pair = pair.as_array()?;
                    let a = pair.first()?.as_u64()? as usize;
                    let b = pair.get(1)?.as_u64()? as usize;
                    (a >= 1 && b >= 1).then(|| [a - 1, b - 1])
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(KeypointSchema { names, skeleton }))
}

fn coco_image_map(
    connection: &Connection,
    coco_images: &[Value],
) -> Result<HashMap<i64, (String, u32, u32)>, String> {
    let mut by_path: HashMap<String, (String, u32, u32)> = HashMap::new();
    let mut by_name: HashMap<String, Vec<(String, u32, u32)>> = HashMap::new();
    for (id, relative, name, width, height) in detection::image_lookup(connection)? {
        by_path.insert(relative.to_lowercase(), (id.clone(), width, height));
        by_name
            .entry(name.to_lowercase())
            .or_default()
            .push((id, width, height));
    }
    let mut map = HashMap::new();
    for item in coco_images {
        let id = integer(item.get("id"), "image id")?;
        let name = item
            .get("file_name")
            .and_then(Value::as_str)
            .ok_or_else(|| "A COCO image has no file_name.".to_owned())?
            .replace('\\', "/");
        let found = by_path.get(&name.to_lowercase()).cloned().or_else(|| {
            let base = Path::new(&name)
                .file_name()?
                .to_string_lossy()
                .to_lowercase();
            match by_name.get(&base)?.as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            }
        });
        if let Some(value) = found {
            map.insert(id, value);
        }
    }
    Ok(map)
}

fn open(root_path: &str) -> Result<Connection, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    let task: String = connection
        .query_row("SELECT task_type FROM project LIMIT 1", [], |row| {
            row.get(0)
        })
        .map_err(|error| format!("Could not read the project task: {error}"))?;
    if task != "keypoints" {
        return Err("This operation is only available in a keypoint project.".to_owned());
    }
    Ok(connection)
}

fn load_schema(connection: &Connection) -> Result<KeypointSchema, String> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [SCHEMA_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read the keypoint names: {error}"))?;
    stored
        .map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| format!("Could not understand the keypoint names: {error}"))
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn save_schema(connection: &Connection, schema: &KeypointSchema) -> Result<(), String> {
    let json = serde_json::to_string(schema)
        .map_err(|error| format!("Could not encode the keypoint names: {error}"))?;
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SCHEMA_KEY, json],
        )
        .map(|_| ())
        .map_err(|error| format!("Could not save the keypoint names: {error}"))
}

fn has_annotations(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM annotations WHERE kind = 'keypoints')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not inspect keypoint objects: {error}"))
}

fn generated_names(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("kpt_{index}")).collect()
}

fn class_indices(classes: &[ClassRecord]) -> HashMap<&str, usize> {
    classes
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.as_str(), index))
        .collect()
}

fn array<'a>(document: &'a Value, name: &str) -> Result<&'a Vec<Value>, String> {
    document
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("COCO JSON has no {name} array."))
}

fn integer(value: Option<&Value>, label: &str) -> Result<i64, String> {
    value
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("COCO {label} must be an integer."))
}

fn number(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(|| "COCO values must be finite numbers.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    fn project_with_image(names: &[&str]) -> (TempDir, String, String, ClassRecord) {
        let directory = TempDir::new().expect("temp");
        ImageBuffer::from_pixel(200, 100, Rgb([1_u8, 2_u8, 3_u8]))
            .save(directory.path().join("frame.jpg"))
            .expect("image");
        let root = directory.path().to_string_lossy().into_owned();
        project::create(&root, "Pose", "keypoints", None).expect("project");
        set_schema(
            &root,
            KeypointSchema {
                names: names.iter().map(|name| (*name).to_owned()).collect(),
                skeleton: vec![[0, 1]],
            },
        )
        .expect("schema");
        let image_id = project::list_images(&root, 10, 0).expect("images")[0]
            .id
            .clone();
        let class = detection::create_class(&root, "person").expect("class");
        (directory, root, image_id, class)
    }

    fn kp(x: f64, y: f64, v: u8) -> Keypoint {
        Keypoint { x, y, v }
    }

    fn geometry(points: Vec<Keypoint>) -> KeypointGeometry {
        KeypointGeometry {
            bbox: BboxGeometry {
                x: 20.0,
                y: 10.0,
                width: 100.0,
                height: 60.0,
            },
            keypoints: points,
        }
    }

    #[test]
    fn objects_round_trip_through_point_edits_and_undo() {
        let (_directory, root, image_id, class) = project_with_image(&["head", "tail"]);
        let created =
            create_annotation(&root, &image_id, &class.id, geometry(vec![])).expect("box");
        assert_eq!(created.geometry.keypoints, vec![Keypoint::default(); 2]);
        let placed = geometry(vec![kp(30.0, 20.0, 2), kp(5.0, 5.0, 0)]);
        update_annotation(&root, &created.id, &class.id, placed.clone(), true).expect("point");
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0].geometry,
            geometry(vec![kp(30.0, 20.0, 2), Keypoint::default()])
        );
        crate::workspace::undo(&root).expect("undo point");
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0]
                .geometry
                .keypoints[0],
            Keypoint::default()
        );
        crate::workspace::undo(&root).expect("undo box");
        assert!(list_annotations(&root, &image_id).expect("list").is_empty());
        crate::workspace::redo(&root).expect("redo box");
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0].id,
            created.id
        );
    }

    #[test]
    fn rejects_points_outside_the_image_and_extra_slots() {
        let (_directory, root, image_id, class) = project_with_image(&["head"]);
        assert!(create_annotation(
            &root,
            &image_id,
            &class.id,
            geometry(vec![kp(201.0, 5.0, 2)])
        )
        .is_err());
        assert!(
            create_annotation(&root, &image_id, &class.id, geometry(vec![kp(1.0, 1.0, 3)]))
                .is_err()
        );
        assert!(create_annotation(
            &root,
            &image_id,
            &class.id,
            geometry(vec![Keypoint::default(); 2])
        )
        .is_err());
    }

    #[test]
    fn schema_grows_freely_but_only_shrinks_without_objects() {
        let (_directory, root, image_id, class) = project_with_image(&["head", "tail"]);
        let created = create_annotation(
            &root,
            &image_id,
            &class.id,
            geometry(vec![kp(30.0, 20.0, 2)]),
        )
        .expect("object");
        let grown = KeypointSchema {
            names: vec!["head".into(), "tail".into(), "fin".into()],
            skeleton: vec![[0, 1]],
        };
        set_schema(&root, grown).expect("append a point");
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0]
                .geometry
                .keypoints
                .len(),
            3
        );
        let shrunk = KeypointSchema {
            names: vec!["head".into()],
            skeleton: vec![[0, 1]],
        };
        assert!(set_schema(&root, shrunk.clone())
            .expect_err("in use")
            .contains("Delete every object"));
        delete_annotation(&root, &created.id).expect("delete");
        assert_eq!(
            set_schema(&root, shrunk).expect("shrink").skeleton,
            Vec::<[usize; 2]>::new()
        );
        assert!(set_schema(
            &root,
            KeypointSchema {
                names: vec!["a".into(), "A".into()],
                skeleton: vec![]
            }
        )
        .is_err());
    }

    #[test]
    fn yolo_pose_export_and_import_preserve_flags_and_geometry() {
        let (directory, root, image_id, class) = project_with_image(&["head", "tail", "fin"]);
        let expected = geometry(vec![
            kp(30.0, 20.0, 2),
            Keypoint::default(),
            kp(100.0, 50.0, 1),
        ]);
        create_annotation(&root, &image_id, &class.id, expected.clone()).expect("object");
        let destination = directory.path().join("pose");
        export_yolo(&root, &destination.to_string_lossy()).expect("export");
        let row = fs::read_to_string(destination.join("frame.txt")).expect("label");
        assert_eq!(row.split_whitespace().count(), 5 + 3 * 3);
        assert!(row.contains(" 0 0 0 "));
        assert!(row.trim_end().ends_with(" 1"));

        let report = import_yolo(&root, &destination.to_string_lossy()).expect("import");
        assert_eq!((report.annotations, report.skipped), (1, 0));
        let imported = &list_annotations(&root, &image_id).expect("list")[0].geometry;
        assert!(
            (imported.bbox.x - 20.0).abs() < 1e-6 && (imported.bbox.width - 100.0).abs() < 1e-6
        );
        for (actual, wanted) in imported.keypoints.iter().zip(&expected.keypoints) {
            assert_eq!(actual.v, wanted.v);
            assert!((actual.x - wanted.x).abs() < 1e-6 && (actual.y - wanted.y).abs() < 1e-6);
        }
    }

    #[test]
    fn yolo_import_infers_point_count_and_reads_flagless_rows() {
        let directory = TempDir::new().expect("temp");
        ImageBuffer::from_pixel(200, 100, Rgb([1_u8, 2_u8, 3_u8]))
            .save(directory.path().join("frame.jpg"))
            .expect("image");
        let root = directory.path().to_string_lossy().into_owned();
        project::create(&root, "Pose", "keypoints", None).expect("project");
        let labels = directory.path().join("labels");
        fs::create_dir(&labels).expect("labels");
        fs::write(labels.join("labels.txt"), "fish\n").expect("classes");
        fs::write(
            labels.join("frame.txt"),
            "0 0.5 0.5 0.5 0.5 0.25 0.5 2 0 0 0\n0 0.5 0.5 0.2 0.2 0.5 0.5 2 0.6 0.5 7\n",
        )
        .expect("rows");
        let report = import_yolo(&root, &labels.to_string_lossy()).expect("import");
        assert_eq!((report.annotations, report.skipped), (1, 1));
        assert_eq!(schema(&root).expect("schema").names, vec!["kpt_0", "kpt_1"]);

        // A [K, 2] file for the same K: (0, 0) is unlabeled, anything else visible.
        fs::write(labels.join("frame.txt"), "0 0.5 0.5 0.5 0.5 0.25 0.5 0 0\n").expect("flagless");
        import_yolo(&root, &labels.to_string_lossy()).expect("flagless import");
        let image_id = project::list_images(&root, 10, 0).expect("images")[0]
            .id
            .clone();
        let points = &list_annotations(&root, &image_id).expect("list")[0]
            .geometry
            .keypoints;
        assert_eq!(points, &vec![kp(50.0, 50.0, 2), Keypoint::default()]);
    }

    #[test]
    fn coco_export_and_import_carry_names_skeleton_and_points() {
        let (directory, root, image_id, class) = project_with_image(&["head", "tail"]);
        let expected = geometry(vec![kp(30.0, 20.0, 2), kp(60.5, 40.25, 1)]);
        create_annotation(&root, &image_id, &class.id, expected.clone()).expect("object");
        let path = directory.path().join("pose.json");
        export_coco(&root, &path.to_string_lossy()).expect("export");
        let document: Value =
            serde_json::from_slice(&fs::read(&path).expect("json")).expect("parse");
        assert_eq!(
            document["categories"][0]["keypoints"],
            json!(["head", "tail"])
        );
        assert_eq!(document["categories"][0]["skeleton"], json!([[1, 2]]));
        assert_eq!(
            document["annotations"][0]["keypoints"],
            json!([30.0, 20.0, 2, 60.5, 40.25, 1])
        );
        assert_eq!(document["annotations"][0]["num_keypoints"], json!(2));

        // A fresh project adopts the names and skeleton from the file.
        let other = TempDir::new().expect("temp");
        ImageBuffer::from_pixel(200, 100, Rgb([1_u8, 2_u8, 3_u8]))
            .save(other.path().join("frame.jpg"))
            .expect("image");
        let other_root = other.path().to_string_lossy().into_owned();
        project::create(&other_root, "Pose", "keypoints", None).expect("project");
        let report = import_coco(&other_root, &path.to_string_lossy()).expect("import");
        assert_eq!(report.annotations, 1);
        assert_eq!(
            schema(&other_root).expect("schema"),
            KeypointSchema {
                names: vec!["head".into(), "tail".into()],
                skeleton: vec![[0, 1]]
            }
        );
        let other_image = project::list_images(&other_root, 10, 0).expect("images")[0]
            .id
            .clone();
        assert_eq!(
            list_annotations(&other_root, &other_image).expect("list")[0].geometry,
            expected
        );

        // A file with a different K is refused before anything changes.
        let (_three_directory, three_root, _, _) = project_with_image(&["a", "b", "c"]);
        assert!(import_coco(&three_root, &path.to_string_lossy())
            .expect_err("count")
            .contains("2 keypoints"));
    }

    #[test]
    fn coco_import_skips_crowds_and_fills_missing_points() {
        let (directory, root, image_id, _) = project_with_image(&["head", "tail"]);
        let source = directory.path().join("in.json");
        fs::write(
            &source,
            r#"{"images": [{"id": 1, "file_name": "frame.jpg"}],
                "categories": [{"id": 3, "name": "person"}],
                "annotations": [
                  {"id": 1, "image_id": 1, "category_id": 3, "bbox": [10, 10, 50, 50]},
                  {"id": 2, "image_id": 1, "category_id": 3, "bbox": [10, 10, 50, 50], "iscrowd": 1},
                  {"id": 3, "image_id": 1, "category_id": 3, "bbox": [10, 10, 50, 50], "keypoints": [500, 5, 2, 20, 20, 2]}
                ]}"#,
        )
        .expect("fixture");
        let report = import_coco(&root, &source.to_string_lossy()).expect("import");
        assert_eq!((report.annotations, report.skipped), (2, 1));
        let objects = list_annotations(&root, &image_id).expect("list");
        assert_eq!(objects[0].geometry.keypoints, vec![Keypoint::default(); 2]);
        assert_eq!(
            objects[1].geometry.keypoints,
            vec![Keypoint::default(), kp(20.0, 20.0, 2)]
        );
    }
}
