use crate::{detection, project};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use uuid::Uuid;
use walkdir::WalkDir;

type IndexedImage = (String, String, String, u32, u32);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolygonPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolygonGeometry {
    pub polygons: Vec<Vec<PolygonPoint>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolygonAnnotationRecord {
    pub id: String,
    pub image_id: String,
    pub class_id: String,
    pub kind: String,
    pub geometry: PolygonGeometry,
    pub is_visible: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolygonDraft {
    pub image_id: String,
    pub class_id: String,
    pub points: Vec<PolygonPoint>,
    pub annotation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PolygonOperation {
    operation_type: String,
    before: Vec<PolygonAnnotationRecord>,
    after: Vec<PolygonAnnotationRecord>,
}

pub fn list_annotations(
    root_path: &str,
    image_id: &str,
) -> Result<Vec<PolygonAnnotationRecord>, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    annotations_for_image(&connection, image_id)
}

pub fn create_annotation(
    root_path: &str,
    image_id: &str,
    class_id: &str,
    geometry: PolygonGeometry,
) -> Result<PolygonAnnotationRecord, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    validate_annotation(&connection, image_id, class_id, &geometry)?;
    let now = project::now_ms()?;
    let record = PolygonAnnotationRecord {
        id: Uuid::new_v4().to_string(),
        image_id: image_id.to_owned(),
        class_id: class_id.to_owned(),
        kind: "polygon".to_owned(),
        geometry,
        is_visible: true,
        created_at_ms: now,
        updated_at_ms: now,
    };
    commit_annotation_change_and_clear_draft(
        &mut connection,
        vec![],
        vec![record.clone()],
        image_id,
    )?;
    Ok(record)
}

pub fn update_annotation(
    root_path: &str,
    annotation_id: &str,
    class_id: &str,
    geometry: PolygonGeometry,
    is_visible: bool,
) -> Result<PolygonAnnotationRecord, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    let before = annotation_by_id(&connection, annotation_id)?
        .ok_or_else(|| "The polygon no longer exists.".to_owned())?;
    validate_annotation(&connection, &before.image_id, class_id, &geometry)?;
    let mut after = before.clone();
    after.class_id = class_id.to_owned();
    after.geometry = geometry;
    after.is_visible = is_visible;
    after.updated_at_ms = project::now_ms()?;
    commit_annotation_change(&mut connection, vec![before], vec![after.clone()])?;
    Ok(after)
}

pub fn append_island(
    root_path: &str,
    annotation_id: &str,
    points: Vec<PolygonPoint>,
) -> Result<PolygonAnnotationRecord, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    let before = annotation_by_id(&connection, annotation_id)?
        .ok_or_else(|| "The polygon receiving this island no longer exists.".to_owned())?;
    let mut after = before.clone();
    after.geometry.polygons.push(points);
    validate_annotation(
        &connection,
        &after.image_id,
        &after.class_id,
        &after.geometry,
    )?;
    after.updated_at_ms = project::now_ms()?;
    commit_annotation_change_and_clear_draft(
        &mut connection,
        vec![before],
        vec![after.clone()],
        &after.image_id,
    )?;
    Ok(after)
}

pub fn delete_annotation(root_path: &str, annotation_id: &str) -> Result<(), String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    let before = annotation_by_id(&connection, annotation_id)?
        .ok_or_else(|| "The polygon no longer exists.".to_owned())?;
    commit_annotation_change(&mut connection, vec![before], vec![])
}

pub fn save_draft(
    root_path: &str,
    draft: Option<PolygonDraft>,
    image_id: &str,
) -> Result<(), String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    if let Some(draft) = draft {
        if draft.image_id != image_id
            || draft.points.is_empty()
            || !class_exists(&connection, &draft.class_id)?
        {
            return Err("The polygon draft is invalid.".to_owned());
        }
        let (width, height) = image_dimensions(&connection, image_id)?;
        if draft
            .points
            .iter()
            .any(|point| !point_in_image(*point, width, height))
        {
            return Err("The polygon draft must remain inside the image.".to_owned());
        }
        if let Some(annotation_id) = &draft.annotation_id {
            let target = annotation_by_id(&connection, annotation_id)?
                .ok_or_else(|| "The polygon receiving this island no longer exists.".to_owned())?;
            if target.image_id != image_id || target.class_id != draft.class_id {
                return Err("An island must belong to the selected polygon object.".to_owned());
            }
        }
        let payload = serde_json::to_string(&draft)
            .map_err(|error| format!("Could not encode the polygon draft: {error}"))?;
        connection.execute(
            "INSERT INTO drafts (image_id, kind, payload_json, updated_at_ms) VALUES (?1, 'polygon', ?2, ?3) ON CONFLICT(image_id) DO UPDATE SET kind = 'polygon', payload_json = excluded.payload_json, updated_at_ms = excluded.updated_at_ms",
            params![image_id, payload, project::now_ms()?],
        ).map(|_| ()).map_err(|error| format!("Could not autosave the polygon draft: {error}"))
    } else {
        clear_draft(&connection, image_id)
    }
}

pub fn load_draft(root_path: &str, image_id: &str) -> Result<Option<PolygonDraft>, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload_json FROM drafts WHERE image_id = ?1 AND kind = 'polygon'",
            [image_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not restore the polygon draft: {error}"))?;
    payload
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|error| format!("Could not understand the saved polygon draft: {error}"))
        })
        .transpose()
}

pub fn export_coco(root_path: &str, destination: &str) -> Result<detection::ImportReport, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    let classes = classes(&connection)?;
    let images = image_lookup(&connection)?;
    let class_ids: HashMap<_, _> = classes
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.as_str(), index + 1))
        .collect();
    let mut output = Vec::new();
    let mut annotation_number = 1;
    for (image_index, (image_id, _, _, _, _)) in images.iter().enumerate() {
        for item in annotations_for_image(&connection, image_id)? {
            let segmentation = item
                .geometry
                .polygons
                .iter()
                .map(|polygon| {
                    polygon
                        .iter()
                        .flat_map(|point| [point.x, point.y])
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let [x, y, width, height] = geometry_bbox(&item.geometry);
            let area = item
                .geometry
                .polygons
                .iter()
                .map(|polygon| polygon_area(polygon))
                .sum::<f64>();
            output.push(json!({
                "id": annotation_number,
                "image_id": image_index + 1,
                "category_id": class_ids[item.class_id.as_str()],
                "segmentation": segmentation,
                "bbox": [x, y, width, height],
                "area": area,
                "iscrowd": 0
            }));
            annotation_number += 1;
        }
    }
    let document = json!({
        "info": {"description": "Exported by yalt"},
        "images": images.iter().enumerate().map(|(index, (_, path, _, width, height))| json!({"id": index + 1, "file_name": path, "width": width, "height": height})).collect::<Vec<_>>(),
        "categories": classes.iter().enumerate().map(|(index, item)| json!({"id": index + 1, "name": item.name, "supercategory": ""})).collect::<Vec<_>>(),
        "annotations": output,
    });
    fs::write(
        destination,
        serde_json::to_vec_pretty(&document)
            .map_err(|error| format!("Could not encode COCO JSON: {error}"))?,
    )
    .map_err(|error| format!("Could not write COCO JSON: {error}"))?;
    Ok(detection::ImportReport {
        annotations: annotation_number - 1,
        matched_images: images.len(),
        skipped: 0,
        classes: classes.len(),
        message: format!("Exported {} COCO polygon objects.", annotation_number - 1),
    })
}

pub fn export_yolo(root_path: &str, destination: &str) -> Result<detection::ImportReport, String> {
    let directory = Path::new(destination);
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create the YOLO export folder: {error}"))?;
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    let classes = classes(&connection)?;
    if classes.is_empty() {
        return Err("Add at least one class before exporting YOLO polygons.".to_owned());
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
    let mut objects = 0;
    let mut polygon_rows = 0;
    let mut annotated_images = 0;
    let mut split_objects = 0;
    for (image_id, relative, _, width, height) in &images {
        let rows = annotations_for_image(&connection, image_id)?;
        if rows.is_empty() {
            continue;
        }
        annotated_images += 1;
        objects += rows.len();
        let mut text = String::new();
        for item in rows {
            let class_index = class_indices
                .get(item.class_id.as_str())
                .ok_or_else(|| "A polygon refers to a missing class.".to_owned())?;
            if item.geometry.polygons.len() > 1 {
                split_objects += 1;
            }
            for polygon in item.geometry.polygons {
                polygon_rows += 1;
                text.push_str(&class_index.to_string());
                for point in polygon {
                    text.push_str(&format!(
                        " {:.8} {:.8}",
                        point.x / *width as f64,
                        point.y / *height as f64
                    ));
                }
                text.push('\n');
            }
        }
        let target = directory.join(with_txt_extension(relative));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        fs::write(&target, text)
            .map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    }
    let island_note = if split_objects > 0 {
        format!(" {split_objects} multi-island objects became separate YOLO rows because TXT does not store shared instance IDs.")
    } else {
        String::new()
    };
    Ok(detection::ImportReport {
        annotations: objects,
        matched_images: annotated_images,
        skipped: 0,
        classes: classes.len(),
        message: format!(
            "Exported {objects} objects as {polygon_rows} YOLO polygon rows for {annotated_images} annotated images.{island_note}"
        ),
    })
}

pub fn import_yolo(root_path: &str, labels_root: &str) -> Result<detection::ImportReport, String> {
    let directory = Path::new(labels_root);
    if !directory.is_dir() {
        return Err("Choose the folder containing YOLO polygon .txt files.".to_owned());
    }
    let labels_file = ["labels.txt", "classes.txt"]
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file());
    if let Some(path) = labels_file {
        detection::import_labels(root_path, &path.to_string_lossy())?;
    }

    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;
    let classes = classes(&connection)?;
    if classes.is_empty() {
        return Err("Load labels.txt or create classes before importing YOLO polygons.".to_owned());
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
            .and_then(|value| value.to_str())
            .unwrap_or(file_name)
            .to_lowercase();
        by_stem.entry(stem).or_default().push(id.clone());
    }

    let now = project::now_ms()?;
    let mut imported: HashMap<String, Vec<PolygonAnnotationRecord>> = HashMap::new();
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
                .and_then(|value| value.to_str())
                .map(|value| !value.eq_ignore_ascii_case("txt"))
                .unwrap_or(true)
        {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
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
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            match parse_yolo_polygon(line, width, height, &classes, &image_id, now) {
                Ok(record) => rows.push(record),
                Err(_) => skipped += 1,
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
    Ok(detection::ImportReport {
        annotations,
        matched_images,
        skipped,
        classes: classes.len(),
        message: format!(
            "Imported {annotations} YOLO polygon objects for {matched_images} images; skipped {skipped} malformed or unmatched rows."
        ),
    })
}

fn parse_yolo_polygon(
    line: &str,
    width: u32,
    height: u32,
    classes: &[detection::ClassRecord],
    image_id: &str,
    now: i64,
) -> Result<PolygonAnnotationRecord, String> {
    let values = line.split_whitespace().collect::<Vec<_>>();
    if values.len() < 7 || values.len() % 2 == 0 {
        return Err(
            "A YOLO polygon row needs a class and at least three coordinate pairs.".to_owned(),
        );
    }
    let class_index = values[0]
        .parse::<usize>()
        .map_err(|_| "The YOLO class index is invalid.".to_owned())?;
    let class = classes
        .get(class_index)
        .ok_or_else(|| "The YOLO class index is outside labels.txt.".to_owned())?;
    let coordinates = values[1..]
        .iter()
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite() && *number >= 0.0 && *number <= 1.0)
                .ok_or_else(|| "YOLO coordinates must be finite values from 0 to 1.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let points = coordinates
        .chunks_exact(2)
        .map(|pair| PolygonPoint {
            x: pair[0] * width as f64,
            y: pair[1] * height as f64,
        })
        .collect::<Vec<_>>();
    validate_polygon(&points, width, height)?;
    Ok(PolygonAnnotationRecord {
        id: Uuid::new_v4().to_string(),
        image_id: image_id.to_owned(),
        class_id: class.id.clone(),
        kind: "polygon".to_owned(),
        geometry: PolygonGeometry {
            polygons: vec![points],
        },
        is_visible: true,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

pub fn import_coco(root_path: &str, source: &str) -> Result<detection::ImportReport, String> {
    let document: Value = serde_json::from_slice(
        &fs::read(source).map_err(|error| format!("Could not read COCO JSON: {error}"))?,
    )
    .map_err(|error| format!("COCO JSON is invalid: {error}"))?;
    let categories = array(&document, "categories")?;
    let coco_images = array(&document, "images")?;
    let coco_annotations = array(&document, "annotations")?;
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    require_segmentation(&connection)?;

    let mut class_records = classes(&connection)?;
    let mut category_map = HashMap::new();
    for category in categories {
        let category_id = integer(category.get("id"), "category id")?;
        let name = category
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "A COCO category has no name.".to_owned())?;
        let class = if let Some(found) = class_records
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(name))
        {
            found.clone()
        } else {
            let created = detection::create_class(root_path, name)?;
            class_records.push(created.clone());
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
    let mut image_map = HashMap::new();
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
            image_map.insert(id, value);
        }
    }

    let now = project::now_ms()?;
    let mut imported: HashMap<String, Vec<PolygonAnnotationRecord>> = HashMap::new();
    let mut skipped = 0;
    for item in coco_annotations {
        let parsed = (|| -> Result<PolygonAnnotationRecord, String> {
            let image_key = integer(item.get("image_id"), "annotation image_id")?;
            let category_key = integer(item.get("category_id"), "annotation category_id")?;
            let (image_id, width, height) = image_map
                .get(&image_key)
                .ok_or_else(|| "image is not in this project".to_owned())?;
            let class_id = category_map
                .get(&category_key)
                .ok_or_else(|| "category is not defined".to_owned())?;
            let segments = item
                .get("segmentation")
                .and_then(Value::as_array)
                .ok_or_else(|| "RLE or missing segmentation is unsupported".to_owned())?;
            let mut polygons = Vec::new();
            for segment in segments {
                let coordinates = segment
                    .as_array()
                    .ok_or_else(|| "RLE segmentation is unsupported".to_owned())?;
                if coordinates.len() < 6 || coordinates.len() % 2 != 0 {
                    continue;
                }
                let points = coordinates
                    .chunks_exact(2)
                    .map(|pair| {
                        Ok(PolygonPoint {
                            x: number(&pair[0])?,
                            y: number(&pair[1])?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if validate_polygon(&points, *width, *height).is_ok() {
                    polygons.push(points);
                }
            }
            let geometry = PolygonGeometry { polygons };
            validate_geometry(&geometry, *width, *height)?;
            Ok(PolygonAnnotationRecord {
                id: Uuid::new_v4().to_string(),
                image_id: image_id.clone(),
                class_id: class_id.clone(),
                kind: "polygon".to_owned(),
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
    let annotation_count = after.len();
    if !before.is_empty() || !after.is_empty() {
        commit_annotation_change(&mut connection, before, after)?;
    }
    Ok(detection::ImportReport {
        annotations: annotation_count,
        matched_images,
        skipped,
        classes: class_records.len(),
        message: format!("Imported {annotation_count} COCO polygon objects for {matched_images} images; skipped {skipped}."),
    })
}

pub(crate) fn is_polygon_operation(json: &str) -> bool {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("operation_type")
                .and_then(Value::as_str)
                .map(|kind| kind == "polygon_annotation")
        })
        .unwrap_or(false)
}

pub(crate) fn apply_polygon_history(
    connection: &mut Connection,
    sequence: i64,
    json: &str,
    undo: bool,
) -> Result<(), String> {
    let operation: PolygonOperation = serde_json::from_str(json)
        .map_err(|error| format!("Could not understand polygon history: {error}"))?;
    let (desired, remove) = if undo {
        (&operation.before, &operation.after)
    } else {
        (&operation.after, &operation.before)
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not apply polygon history: {error}"))?;
    for item in remove {
        transaction
            .execute("DELETE FROM annotations WHERE id = ?1", [&item.id])
            .map_err(|error| format!("Could not remove a polygon during undo: {error}"))?;
    }
    for item in desired {
        upsert_annotation(&transaction, item)?;
    }
    transaction
        .execute(
            "UPDATE operation_history SET applied = ?1 WHERE sequence = ?2",
            params![!undo, sequence],
        )
        .map_err(|error| format!("Could not update polygon history: {error}"))?;
    touch_project(&transaction, project::now_ms()?)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit polygon history: {error}"))
}

fn commit_annotation_change(
    connection: &mut Connection,
    before: Vec<PolygonAnnotationRecord>,
    after: Vec<PolygonAnnotationRecord>,
) -> Result<(), String> {
    commit_annotation_change_inner(connection, before, after, None)
}

fn commit_annotation_change_and_clear_draft(
    connection: &mut Connection,
    before: Vec<PolygonAnnotationRecord>,
    after: Vec<PolygonAnnotationRecord>,
    image_id: &str,
) -> Result<(), String> {
    commit_annotation_change_inner(connection, before, after, Some(image_id))
}

fn commit_annotation_change_inner(
    connection: &mut Connection,
    before: Vec<PolygonAnnotationRecord>,
    after: Vec<PolygonAnnotationRecord>,
    draft_image_id: Option<&str>,
) -> Result<(), String> {
    let operation = PolygonOperation {
        operation_type: "polygon_annotation".to_owned(),
        before: before.clone(),
        after: after.clone(),
    };
    let payload = serde_json::to_string(&operation)
        .map_err(|error| format!("Could not encode polygon history: {error}"))?;
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not autosave the polygon: {error}"))?;
    for item in &before {
        transaction
            .execute("DELETE FROM annotations WHERE id = ?1", [&item.id])
            .map_err(|error| format!("Could not replace a polygon: {error}"))?;
    }
    for item in &after {
        upsert_annotation(&transaction, item)?;
    }
    if let Some(image_id) = draft_image_id {
        transaction
            .execute(
                "DELETE FROM drafts WHERE image_id = ?1 AND kind = 'polygon'",
                [image_id],
            )
            .map_err(|error| format!("Could not finish the polygon draft: {error}"))?;
    }
    transaction
        .execute("DELETE FROM operation_history WHERE applied = 0", [])
        .map_err(|error| format!("Could not clear obsolete redo history: {error}"))?;
    transaction.execute("INSERT INTO operation_history (operation_json, created_at_ms, applied) VALUES (?1, ?2, 1)", params![payload, now])
        .map_err(|error| format!("Could not record polygon history: {error}"))?;
    touch_project(&transaction, now)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit polygon autosave: {error}"))
}

fn annotations_for_image(
    connection: &Connection,
    image_id: &str,
) -> Result<Vec<PolygonAnnotationRecord>, String> {
    let mut statement = connection.prepare("SELECT id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms FROM annotations WHERE image_id = ?1 AND kind = 'polygon' ORDER BY created_at_ms, rowid")
        .map_err(|error| format!("Could not prepare polygons: {error}"))?;
    let rows = statement
        .query_map([image_id], annotation_from_row)
        .map_err(|error| format!("Could not load polygons: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read a polygon: {error}"))
}

fn annotation_by_id(
    connection: &Connection,
    id: &str,
) -> Result<Option<PolygonAnnotationRecord>, String> {
    connection.query_row("SELECT id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms FROM annotations WHERE id = ?1 AND kind = 'polygon'", [id], annotation_from_row)
        .optional().map_err(|error| format!("Could not find the polygon: {error}"))
}

fn annotation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PolygonAnnotationRecord> {
    let json: String = row.get(4)?;
    let geometry = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(PolygonAnnotationRecord {
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
    item: &PolygonAnnotationRecord,
) -> Result<(), String> {
    let geometry = serde_json::to_string(&item.geometry)
        .map_err(|error| format!("Could not encode polygon geometry: {error}"))?;
    transaction.execute("INSERT INTO annotations (id, image_id, class_id, kind, geometry_json, is_visible, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, 'polygon', ?4, ?5, ?6, ?7) ON CONFLICT(id) DO UPDATE SET image_id = excluded.image_id, class_id = excluded.class_id, kind = 'polygon', geometry_json = excluded.geometry_json, is_visible = excluded.is_visible, updated_at_ms = excluded.updated_at_ms", params![item.id, item.image_id, item.class_id, geometry, item.is_visible, item.created_at_ms, item.updated_at_ms])
        .map(|_| ()).map_err(|error| format!("Could not save a polygon: {error}"))
}

fn validate_annotation(
    connection: &Connection,
    image_id: &str,
    class_id: &str,
    geometry: &PolygonGeometry,
) -> Result<(), String> {
    if !class_exists(connection, class_id)? {
        return Err("Choose a valid class before drawing a polygon.".to_owned());
    }
    let (width, height) = image_dimensions(connection, image_id)?;
    validate_geometry(geometry, width, height)
}

fn validate_geometry(geometry: &PolygonGeometry, width: u32, height: u32) -> Result<(), String> {
    if geometry.polygons.is_empty() {
        return Err("A polygon object needs at least one contour.".to_owned());
    }
    for polygon in &geometry.polygons {
        validate_polygon(polygon, width, height)?;
    }
    Ok(())
}

fn validate_polygon(points: &[PolygonPoint], width: u32, height: u32) -> Result<(), String> {
    if points.len() < 3
        || points
            .iter()
            .any(|point| !point_in_image(*point, width, height))
        || polygon_area(points) < 0.5
    {
        return Err(
            "Each polygon needs three points, non-zero area, and must remain inside the image."
                .to_owned(),
        );
    }
    Ok(())
}

fn point_in_image(point: PolygonPoint, width: u32, height: u32) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && point.x >= 0.0
        && point.y >= 0.0
        && point.x <= width as f64
        && point.y <= height as f64
}

fn polygon_area(points: &[PolygonPoint]) -> f64 {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let next = points[(index + 1) % points.len()];
            point.x * next.y - next.x * point.y
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

fn geometry_bbox(geometry: &PolygonGeometry) -> [f64; 4] {
    let mut points = geometry.polygons.iter().flatten();
    let first = *points.next().expect("validated polygon geometry");
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x, first.y, first.x, first.y);
    for point in points {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    [min_x, min_y, max_x - min_x, max_y - min_y]
}

fn image_dimensions(connection: &Connection, image_id: &str) -> Result<(u32, u32), String> {
    connection
        .query_row(
            "SELECT width, height FROM images WHERE id = ?1 AND status = 'active'",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Could not find the image: {error}"))?
        .ok_or_else(|| "The image is not available for annotation.".to_owned())
}

fn class_exists(connection: &Connection, class_id: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM classes WHERE id = ?1)",
            [class_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not find the class: {error}"))
}

fn classes(connection: &Connection) -> Result<Vec<detection::ClassRecord>, String> {
    let mut statement = connection
        .prepare("SELECT id, name, position, color, shortcut FROM classes ORDER BY position")
        .map_err(|error| format!("Could not prepare classes: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(detection::ClassRecord {
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

fn image_lookup(connection: &Connection) -> Result<Vec<IndexedImage>, String> {
    let mut statement = connection.prepare("SELECT id, relative_path, file_name, width, height FROM images WHERE status = 'active' ORDER BY sort_order")
        .map_err(|error| format!("Could not inspect project images: {error}"))?;
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

fn require_segmentation(connection: &Connection) -> Result<(), String> {
    let task: String = connection
        .query_row("SELECT task_type FROM project LIMIT 1", [], |row| {
            row.get(0)
        })
        .map_err(|error| format!("Could not read the project task: {error}"))?;
    if task != "segmentation" {
        return Err(
            "This operation is only available in a polygon segmentation project.".to_owned(),
        );
    }
    Ok(())
}

fn clear_draft(connection: &Connection, image_id: &str) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM drafts WHERE image_id = ?1 AND kind = 'polygon'",
            [image_id],
        )
        .map(|_| ())
        .map_err(|error| format!("Could not clear the polygon draft: {error}"))
}

fn touch_project(transaction: &Transaction<'_>, now: i64) -> Result<(), String> {
    transaction
        .execute("UPDATE project SET updated_at_ms = ?1", [now])
        .map(|_| ())
        .map_err(|error| format!("Could not update project state: {error}"))
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
        .ok_or_else(|| format!("Invalid COCO {label}."))
}

fn number(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(|| "Invalid COCO coordinate.".to_owned())
}

fn with_txt_extension(relative: &str) -> String {
    let mut path = std::path::PathBuf::from(relative);
    path.set_extension("txt");
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    #[test]
    fn polygon_area_and_bbox_cover_multiple_contours() {
        let geometry = PolygonGeometry {
            polygons: vec![
                vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)],
                vec![p(20.0, 20.0), p(25.0, 20.0), p(20.0, 25.0)],
            ],
        };
        assert_eq!(polygon_area(&geometry.polygons[0]), 100.0);
        assert_eq!(geometry_bbox(&geometry), [0.0, 0.0, 25.0, 25.0]);
    }

    #[test]
    fn rejects_degenerate_polygon() {
        assert!(validate_polygon(&[p(0.0, 0.0), p(1.0, 1.0), p(2.0, 2.0)], 10, 10).is_err());
    }

    #[test]
    fn annotations_drafts_and_history_round_trip() {
        let (_directory, root, image_id) = project_with_image();
        let class = detection::create_class(&root, "person").expect("class");
        let draft = PolygonDraft {
            image_id: image_id.clone(),
            class_id: class.id.clone(),
            points: vec![p(10.0, 10.0), p(60.0, 10.0)],
            annotation_id: None,
        };
        save_draft(&root, Some(draft.clone()), &image_id).expect("save draft");
        assert_eq!(
            load_draft(&root, &image_id).expect("load draft"),
            Some(draft)
        );
        let geometry = PolygonGeometry {
            polygons: vec![vec![p(10.0, 10.0), p(60.0, 10.0), p(30.0, 50.0)]],
        };
        let created =
            create_annotation(&root, &image_id, &class.id, geometry.clone()).expect("create");
        assert!(load_draft(&root, &image_id).expect("cleared").is_none());
        assert_eq!(
            list_annotations(&root, &image_id).expect("list")[0].geometry,
            geometry
        );
        assert_eq!(
            crate::workspace::undo(&root).expect("undo"),
            Some(image_id.clone())
        );
        assert!(list_annotations(&root, &image_id)
            .expect("empty")
            .is_empty());
        assert_eq!(
            crate::workspace::redo(&root).expect("redo"),
            Some(image_id.clone())
        );
        assert_eq!(
            list_annotations(&root, &image_id).expect("restored")[0].id,
            created.id
        );
        let island = vec![p(80.0, 20.0), p(100.0, 20.0), p(90.0, 40.0)];
        save_draft(
            &root,
            Some(PolygonDraft {
                image_id: image_id.clone(),
                class_id: class.id,
                points: island.clone(),
                annotation_id: Some(created.id.clone()),
            }),
            &image_id,
        )
        .expect("island draft");
        let updated = append_island(&root, &created.id, island).expect("append island");
        assert_eq!(updated.geometry.polygons.len(), 2);
        assert!(load_draft(&root, &image_id)
            .expect("draft cleared")
            .is_none());
        assert_eq!(
            crate::workspace::undo(&root).expect("undo island"),
            Some(image_id.clone())
        );
        assert_eq!(
            list_annotations(&root, &image_id).expect("one contour")[0]
                .geometry
                .polygons
                .len(),
            1
        );
    }

    #[test]
    fn coco_golden_and_export_preserve_multiple_contours() {
        let (directory, root, image_id) = project_with_image();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/coco_polygon.json");
        let report = import_coco(&root, &fixture.to_string_lossy()).expect("golden import");
        assert_eq!(report.annotations, 1);
        let imported = list_annotations(&root, &image_id).expect("polygons");
        assert_eq!(imported[0].geometry.polygons.len(), 2);
        let destination = directory.path().join("exported.json");
        export_coco(&root, &destination.to_string_lossy()).expect("export");
        let exported: Value =
            serde_json::from_slice(&fs::read(destination).expect("json")).expect("document");
        assert_eq!(
            exported["annotations"][0]["segmentation"]
                .as_array()
                .expect("segments")
                .len(),
            2
        );
        assert_eq!(
            exported["annotations"][0]["bbox"],
            json!([20.0, 10.0, 100.0, 40.0])
        );
    }

    #[test]
    fn yolo_export_and_import_normalize_each_island_as_a_row() {
        let (directory, root, image_id) = project_with_image();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/coco_polygon.json");
        import_coco(&root, &fixture.to_string_lossy()).expect("import");
        let destination = directory.path().join("yolo-polygons");
        let report = export_yolo(&root, &destination.to_string_lossy()).expect("export");
        let rows = fs::read_to_string(destination.join("frame.txt")).expect("labels");
        assert_eq!(rows.lines().count(), 2);
        assert!(rows
            .lines()
            .all(|line| line.split_whitespace().next() == Some("0")));
        assert_eq!(report.annotations, 1);
        assert!(report.message.contains("separate YOLO rows"));

        let existing = list_annotations(&root, &image_id).expect("existing polygons");
        delete_annotation(&root, &existing[0].id).expect("delete existing polygon");
        let imported = import_yolo(&root, &destination.to_string_lossy()).expect("YOLO import");
        assert_eq!(imported.annotations, 2);
        assert_eq!(imported.skipped, 0);
        assert_eq!(
            list_annotations(&root, &image_id).expect("polygons").len(),
            2
        );
    }

    #[test]
    fn malformed_coco_polygons_are_counted_in_the_import_report() {
        let (directory, root, image_id) = project_with_image();
        let source = directory.path().join("malformed.json");
        fs::write(
            &source,
            r#"{
              "images": [{"id": 1, "file_name": "frame.jpg"}],
              "categories": [{"id": 1, "name": "person"}],
              "annotations": [{"id": 1, "image_id": 1, "category_id": 1, "segmentation": [[1, 2, 3]]}]
            }"#,
        )
        .expect("fixture");

        let report = import_coco(&root, &source.to_string_lossy()).expect("import report");
        assert_eq!(report.skipped, 1);
        assert!(report.message.contains("skipped 1"));
        assert!(list_annotations(&root, &image_id)
            .expect("annotations")
            .is_empty());
    }

    fn project_with_image() -> (TempDir, String, String) {
        let directory = TempDir::new().expect("temp");
        ImageBuffer::from_pixel(200, 100, Rgb([1_u8, 2_u8, 3_u8]))
            .save(directory.path().join("frame.jpg"))
            .expect("image");
        let root = directory.path().to_string_lossy().into_owned();
        project::create(&root, "Segmentation", "segmentation", None).expect("project");
        let image_id = project::list_images(&root, 10, 0).expect("images")[0]
            .id
            .clone();
        (directory, root, image_id)
    }

    fn p(x: f64, y: f64) -> PolygonPoint {
        PolygonPoint { x, y }
    }
}
