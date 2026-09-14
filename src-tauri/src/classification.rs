use crate::detection::{self, ClassRecord};
use crate::project;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationState {
    pub classes: Vec<ClassRecord>,
    pub image_class_ids: Vec<String>,
    pub labeled_image_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationReport {
    pub assignments: usize,
    pub matched_images: usize,
    pub skipped: usize,
    pub classes: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClassificationOperation {
    operation_type: String,
    image_id: String,
    before_class_ids: Vec<String>,
    after_class_ids: Vec<String>,
    from_relative_path: String,
    to_relative_path: String,
    original_relative_path: Option<String>,
}

struct PendingClassificationMove {
    id: String,
    operation: ClassificationOperation,
    operation_json: String,
    target_class_ids: Vec<String>,
    history_sequence: Option<i64>,
    desired_applied: Option<bool>,
}

pub fn state(root_path: &str, image_id: Option<&str>) -> Result<ClassificationState, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_classification(&connection)?;
    state_from_connection(&connection, image_id)
}

pub fn set_image_classes(
    root_path: &str,
    image_id: &str,
    class_ids: Vec<String>,
) -> Result<ClassificationState, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    let mode = require_classification(&connection)?;
    let class_ids = validate_class_ids(&connection, class_ids)?;
    if mode == "single" && class_ids.len() > 1 {
        return Err("Single-label projects allow one class per image.".to_owned());
    }

    let (current_path, original_path): (String, Option<String>) = connection
        .query_row(
            "SELECT relative_path, classification_original_path FROM images WHERE id = ?1 AND status = 'active'",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Could not find the image to classify: {error}"))?
        .ok_or_else(|| "Only an available image can be classified.".to_owned())?;
    let before = image_class_ids(&connection, image_id)?;
    if before == class_ids {
        return state_from_connection(&connection, Some(image_id));
    }

    let mut operation = ClassificationOperation {
        operation_type: "classification".to_owned(),
        image_id: image_id.to_owned(),
        before_class_ids: before,
        after_class_ids: class_ids.clone(),
        from_relative_path: current_path.clone(),
        to_relative_path: current_path.clone(),
        original_relative_path: original_path,
    };

    if mode == "single" {
        if operation.original_relative_path.is_none() {
            operation.original_relative_path = Some(current_path.clone());
        }
        operation.to_relative_path = classification_destination(
            &root,
            &connection,
            &current_path,
            operation.original_relative_path.as_deref(),
            class_ids.first().map(String::as_str),
            image_id,
        )?;
    }

    if operation.from_relative_path == operation.to_relative_path {
        apply_database_operation(&mut connection, &operation, true, None)?;
    } else {
        perform_new_move(&root, &mut connection, operation)?;
    }
    state_from_connection(&connection, Some(image_id))
}

pub fn is_classification_operation(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| value.get("operation_type")?.as_str().map(str::to_owned))
        .as_deref()
        == Some("classification")
}

pub fn apply_history(
    root: &Path,
    connection: &mut Connection,
    sequence: i64,
    json: &str,
    undo: bool,
) -> Result<(), String> {
    let operation: ClassificationOperation = serde_json::from_str(json)
        .map_err(|error| format!("Could not understand classification history: {error}"))?;
    let (from_path, to_path, target_ids, desired_applied) = if undo {
        (
            operation.to_relative_path.clone(),
            operation.from_relative_path.clone(),
            operation.before_class_ids.clone(),
            false,
        )
    } else {
        (
            operation.from_relative_path.clone(),
            operation.to_relative_path.clone(),
            operation.after_class_ids.clone(),
            true,
        )
    };
    if from_path == to_path {
        return apply_database_operation(connection, &operation, !undo, Some(sequence));
    }
    let replay = ClassificationOperation {
        from_relative_path: from_path,
        to_relative_path: to_path,
        ..operation
    };
    let pending = PendingClassificationMove {
        id: Uuid::new_v4().to_string(),
        operation_json: json.to_owned(),
        operation: replay,
        target_class_ids: target_ids,
        history_sequence: Some(sequence),
        desired_applied: Some(desired_applied),
    };
    insert_pending(connection, &pending)?;
    if let Err(error) = move_path(
        root,
        &pending.operation.from_relative_path,
        &pending.operation.to_relative_path,
    ) {
        let _ = connection.execute(
            "DELETE FROM classification_file_operations WHERE id = ?1",
            [&pending.id],
        );
        return Err(error);
    }
    finalize_pending(connection, &pending)
}

pub fn recover_pending(root: &Path, connection: &mut Connection) -> Result<(), String> {
    let pending = {
        let mut statement = connection
            .prepare(
                "SELECT id, operation_json, target_class_ids, history_sequence, desired_applied\n                 FROM classification_file_operations ORDER BY created_at_ms",
            )
            .map_err(|error| format!("Could not inspect interrupted classification moves: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                let operation_json: String = row.get(1)?;
                let target_json: String = row.get(2)?;
                Ok((
                    row.get::<_, String>(0)?,
                    operation_json,
                    target_json,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?.map(|value| value != 0),
                ))
            })
            .map_err(|error| format!("Could not read interrupted classification moves: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            format!("Could not read an interrupted classification move: {error}")
        })?
    };
    for (id, operation_json, target_json, history_sequence, desired_applied) in pending {
        let operation: ClassificationOperation = serde_json::from_str(&operation_json)
            .map_err(|error| format!("Could not recover classification move data: {error}"))?;
        let target_class_ids: Vec<String> = serde_json::from_str(&target_json)
            .map_err(|error| format!("Could not recover classification labels: {error}"))?;
        let item = PendingClassificationMove {
            id,
            operation,
            operation_json,
            target_class_ids,
            history_sequence,
            desired_applied,
        };
        let source = relative_path(root, &item.operation.from_relative_path)?;
        let destination = relative_path(root, &item.operation.to_relative_path)?;
        match (source.exists(), destination.exists()) {
            (true, false) => {
                connection
                    .execute(
                        "DELETE FROM classification_file_operations WHERE id = ?1",
                        [&item.id],
                    )
                    .map_err(|error| {
                        format!("Could not clear an unstarted classification move: {error}")
                    })?;
            }
            (false, true) => finalize_pending(connection, &item)?,
            (true, true) => {
                return Err(format!(
                    "Classification recovery stopped because both {} and {} exist.",
                    source.display(),
                    destination.display()
                ))
            }
            (false, false) => {
                return Err(format!(
                    "Classification recovery stopped because neither {} nor {} exists.",
                    source.display(),
                    destination.display()
                ))
            }
        }
    }
    Ok(())
}

pub fn export_csv(root_path: &str, destination_path: &str) -> Result<ClassificationReport, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_classification(&connection)?;
    let rows = assignment_rows(&connection)?;
    let mut output = String::from("relative_path,class_name_1,class_name_2\n");
    let mut assignments = 0;
    for (path, names) in &rows {
        assignments += names.len();
        let mut fields = vec![path.clone()];
        fields.extend(names.iter().cloned());
        output.push_str(
            &fields
                .iter()
                .map(|field| csv_field(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        output.push('\n');
    }
    fs::write(destination_path, output)
        .map_err(|error| format!("Could not write classification CSV: {error}"))?;
    Ok(ClassificationReport {
        assignments,
        matched_images: rows.len(),
        skipped: 0,
        classes: class_records(&connection)?.len(),
        message: format!("Exported labels for {} images to CSV.", rows.len()),
    })
}

pub fn import_csv(root_path: &str, source_path: &str) -> Result<ClassificationReport, String> {
    let text = fs::read_to_string(source_path)
        .map_err(|error| format!("Could not read classification CSV: {error}"))?;
    let rows = parse_csv(&text)?;
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_classification(&connection)?;
    let lookup = image_lookup(&connection)?;
    drop(connection);
    let mut matched = 0;
    let mut skipped = 0;
    let mut assignments = 0;
    let mut class_names = HashSet::new();
    for (index, row) in rows.into_iter().enumerate() {
        if row.is_empty() || row.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        if index == 0
            && matches!(
                row[0].trim().to_ascii_lowercase().as_str(),
                "relative_path" | "path" | "file"
            )
        {
            continue;
        }
        let Some(image_id) = resolve_image(&lookup, row[0].trim()) else {
            skipped += 1;
            continue;
        };
        let names = row
            .iter()
            .skip(1)
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        let ids = ensure_classes(root_path, &names)?;
        match set_image_classes(root_path, &image_id, ids) {
            Ok(_) => {
                matched += 1;
                assignments += names.len();
                class_names.extend(names.into_iter().map(str::to_lowercase));
            }
            Err(_) => skipped += 1,
        }
    }
    Ok(ClassificationReport {
        assignments,
        matched_images: matched,
        skipped,
        classes: class_names.len(),
        message: format!(
            "Imported {assignments} assignments for {matched} images; skipped {skipped}."
        ),
    })
}

pub fn export_class_directories(
    root_path: &str,
    destination_path: &str,
) -> Result<ClassificationReport, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    require_classification(&connection)?;
    let destination = PathBuf::from(destination_path);
    if destination.starts_with(&root) {
        return Err(
            "Choose a class-directory export location outside the project dataset.".to_owned(),
        );
    }
    if destination.exists() {
        let mut entries = fs::read_dir(&destination)
            .map_err(|error| format!("Could not inspect export folder: {error}"))?;
        if entries.next().is_some() {
            return Err("Choose a new or empty folder for class-directory export.".to_owned());
        }
    }
    fs::create_dir_all(&destination)
        .map_err(|error| format!("Could not create class-directory export: {error}"))?;
    let mut statement = connection
        .prepare("SELECT i.id, i.relative_path, i.file_name, c.name FROM image_classes ic JOIN images i ON i.id = ic.image_id JOIN classes c ON c.id = ic.class_id WHERE i.status = 'active' ORDER BY i.sort_order, c.position")
        .map_err(|error| format!("Could not prepare class-directory export: {error}"))?;
    let records = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("Could not read class-directory export: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read a classification assignment: {error}"))?;
    let mut copied = 0;
    for (image_id, relative, file_name, class_name) in &records {
        let class_directory = destination.join(class_directory_name(class_name)?);
        fs::create_dir_all(&class_directory)
            .map_err(|error| format!("Could not create {}: {error}", class_directory.display()))?;
        let source = project::safe_image_path(&root, relative)?;
        let target = available_external_path(&class_directory, file_name, image_id);
        fs::copy(&source, &target)
            .map_err(|error| format!("Could not copy {}: {error}", source.display()))?;
        copied += 1;
    }
    Ok(ClassificationReport {
        assignments: copied,
        matched_images: records
            .iter()
            .map(|row| &row.0)
            .collect::<HashSet<_>>()
            .len(),
        skipped: 0,
        classes: records
            .iter()
            .map(|row| row.3.to_lowercase())
            .collect::<HashSet<_>>()
            .len(),
        message: format!("Exported {copied} labeled image copies into class directories."),
    })
}

pub fn import_class_directories(
    root_path: &str,
    source_path: &str,
) -> Result<ClassificationReport, String> {
    let source = fs::canonicalize(source_path)
        .map_err(|error| format!("Could not open class directories: {error}"))?;
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    let mode = require_classification(&connection)?;
    if mode != "single" {
        return Err(
            "Class-directory import is only available for single-label projects.".to_owned(),
        );
    }
    let lookup = image_lookup(&connection)?;
    drop(connection);
    let mut matched = 0;
    let mut skipped = 0;
    let mut classes = HashSet::new();
    for entry in fs::read_dir(&source)
        .map_err(|error| format!("Could not list class directories: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Could not read a class directory: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("Could not inspect a class directory: {error}"))?
            .is_dir()
        {
            continue;
        }
        let class_name = entry.file_name().to_string_lossy().into_owned();
        if class_name.starts_with('.') || class_name.eq_ignore_ascii_case("deleted") {
            continue;
        }
        let class_id = ensure_classes(root_path, &[class_name.as_str()])?.remove(0);
        classes.insert(class_name.to_lowercase());
        for image in WalkDir::new(entry.path())
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !image.file_type().is_file() || !supported_image(image.path()) {
                continue;
            }
            let relative_to_root = image.path().strip_prefix(&root).ok().map(normalize_path);
            let key = relative_to_root
                .as_deref()
                .unwrap_or_else(|| image.file_name().to_str().unwrap_or(""));
            let Some(image_id) = resolve_image(&lookup, key) else {
                skipped += 1;
                continue;
            };
            match set_image_classes(root_path, &image_id, vec![class_id.clone()]) {
                Ok(_) => matched += 1,
                Err(_) => skipped += 1,
            }
        }
    }
    Ok(ClassificationReport {
        assignments: matched,
        matched_images: matched,
        skipped,
        classes: classes.len(),
        message: format!("Imported {matched} class-directory assignments; skipped {skipped}."),
    })
}

fn state_from_connection(
    connection: &Connection,
    image_id: Option<&str>,
) -> Result<ClassificationState, String> {
    let classes = class_records(connection)?;
    let image_class_ids = match image_id {
        Some(id) => image_class_ids(connection, id)?,
        None => Vec::new(),
    };
    let labeled_image_count = connection
        .query_row(
            "SELECT COUNT(DISTINCT image_id) FROM image_classes",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not count classified images: {error}"))?;
    Ok(ClassificationState {
        classes,
        image_class_ids,
        labeled_image_count,
    })
}

fn class_records(connection: &Connection) -> Result<Vec<ClassRecord>, String> {
    let mut statement = connection
        .prepare("SELECT id, name, position, color, shortcut FROM classes ORDER BY position")
        .map_err(|error| format!("Could not prepare classification classes: {error}"))?;
    let records = statement
        .query_map([], |row| {
            Ok(ClassRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
                color: row.get(3)?,
                shortcut: row.get(4)?,
            })
        })
        .map_err(|error| format!("Could not load classification classes: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read a classification class: {error}"))?;
    Ok(records)
}

fn image_class_ids(connection: &Connection, image_id: &str) -> Result<Vec<String>, String> {
    let mut statement = connection.prepare("SELECT ic.class_id FROM image_classes ic JOIN classes c ON c.id = ic.class_id WHERE ic.image_id = ?1 ORDER BY c.position")
        .map_err(|error| format!("Could not prepare image labels: {error}"))?;
    let ids = statement
        .query_map([image_id], |row| row.get(0))
        .map_err(|error| format!("Could not load image labels: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read an image label: {error}"))?;
    Ok(ids)
}

fn validate_class_ids(
    connection: &Connection,
    class_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut unique = HashSet::new();
    let mut ids = class_ids
        .into_iter()
        .filter(|id| unique.insert(id.clone()))
        .collect::<Vec<_>>();
    let positions = class_records(connection)?
        .into_iter()
        .map(|class| (class.id, class.position))
        .collect::<HashMap<_, _>>();
    if ids.iter().any(|id| !positions.contains_key(id)) {
        return Err("One of the selected classes no longer exists.".to_owned());
    }
    ids.sort_by_key(|id| positions[id]);
    Ok(ids)
}

fn apply_database_operation(
    connection: &mut Connection,
    operation: &ClassificationOperation,
    forward: bool,
    sequence: Option<i64>,
) -> Result<(), String> {
    let ids = if forward {
        &operation.after_class_ids
    } else {
        &operation.before_class_ids
    };
    let path = if forward {
        &operation.to_relative_path
    } else {
        &operation.from_relative_path
    };
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start classification update: {error}"))?;
    replace_assignments(&transaction, &operation.image_id, ids, now)?;
    transaction.execute(
        "UPDATE images SET relative_path = ?1, file_name = ?2, classification_original_path = ?3, updated_at_ms = ?4 WHERE id = ?5",
        params![path, file_name(path), operation.original_relative_path, now, operation.image_id],
    ).map_err(|error| format!("Could not update the classified image: {error}"))?;
    record_history(&transaction, operation, forward, sequence, now)?;
    transaction
        .execute("UPDATE project SET updated_at_ms = ?1", [now])
        .map_err(|error| format!("Could not update project state: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Could not save image classification: {error}"))
}

fn perform_new_move(
    root: &Path,
    connection: &mut Connection,
    operation: ClassificationOperation,
) -> Result<(), String> {
    let operation_json = serde_json::to_string(&operation)
        .map_err(|error| format!("Could not record classification move: {error}"))?;
    let pending = PendingClassificationMove {
        id: Uuid::new_v4().to_string(),
        target_class_ids: operation.after_class_ids.clone(),
        operation,
        operation_json,
        history_sequence: None,
        desired_applied: None,
    };
    insert_pending(connection, &pending)?;
    if let Err(error) = move_path(
        root,
        &pending.operation.from_relative_path,
        &pending.operation.to_relative_path,
    ) {
        let _ = connection.execute(
            "DELETE FROM classification_file_operations WHERE id = ?1",
            [&pending.id],
        );
        return Err(error);
    }
    finalize_pending(connection, &pending)
}

fn insert_pending(
    connection: &Connection,
    pending: &PendingClassificationMove,
) -> Result<(), String> {
    let class_json = serde_json::to_string(&pending.target_class_ids)
        .map_err(|error| format!("Could not journal classification labels: {error}"))?;
    connection.execute(
        "INSERT INTO classification_file_operations (id, image_id, from_relative_path, to_relative_path, target_class_ids, operation_json, history_sequence, desired_applied, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![pending.id, pending.operation.image_id, pending.operation.from_relative_path, pending.operation.to_relative_path, class_json, pending.operation_json, pending.history_sequence, pending.desired_applied, project::now_ms()?],
    ).map(|_| ()).map_err(|error| format!("Could not prepare recoverable classification move: {error}"))
}

fn finalize_pending(
    connection: &mut Connection,
    pending: &PendingClassificationMove,
) -> Result<(), String> {
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not finalize classification move: {error}"))?;
    replace_assignments(
        &transaction,
        &pending.operation.image_id,
        &pending.target_class_ids,
        now,
    )?;
    transaction.execute(
        "UPDATE images SET relative_path = ?1, file_name = ?2, classification_original_path = ?3, updated_at_ms = ?4 WHERE id = ?5",
        params![pending.operation.to_relative_path, file_name(&pending.operation.to_relative_path), pending.operation.original_relative_path, now, pending.operation.image_id],
    ).map_err(|error| format!("Could not update moved classification image: {error}"))?;
    record_history(
        &transaction,
        &pending.operation,
        pending.desired_applied.unwrap_or(true),
        pending.history_sequence,
        now,
    )?;
    transaction
        .execute("UPDATE project SET updated_at_ms = ?1", [now])
        .map_err(|error| format!("Could not update project state: {error}"))?;
    transaction
        .execute(
            "DELETE FROM classification_file_operations WHERE id = ?1",
            [&pending.id],
        )
        .map_err(|error| format!("Could not complete classification move journal: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit classification move: {error}"))
}

fn replace_assignments(
    transaction: &Transaction<'_>,
    image_id: &str,
    ids: &[String],
    now: i64,
) -> Result<(), String> {
    transaction
        .execute("DELETE FROM image_classes WHERE image_id = ?1", [image_id])
        .map_err(|error| format!("Could not replace image labels: {error}"))?;
    for id in ids {
        transaction
            .execute(
                "INSERT INTO image_classes (image_id, class_id, created_at_ms) VALUES (?1, ?2, ?3)",
                params![image_id, id, now],
            )
            .map_err(|error| format!("Could not save an image label: {error}"))?;
    }
    Ok(())
}

fn record_history(
    transaction: &Transaction<'_>,
    operation: &ClassificationOperation,
    applied: bool,
    sequence: Option<i64>,
    now: i64,
) -> Result<(), String> {
    if let Some(sequence) = sequence {
        transaction
            .execute(
                "UPDATE operation_history SET applied = ?1 WHERE sequence = ?2",
                params![applied, sequence],
            )
            .map_err(|error| format!("Could not update classification history: {error}"))?;
    } else {
        let json = serde_json::to_string(operation)
            .map_err(|error| format!("Could not encode classification history: {error}"))?;
        transaction
            .execute("DELETE FROM operation_history WHERE applied = 0", [])
            .map_err(|error| format!("Could not clear obsolete redo history: {error}"))?;
        transaction.execute("INSERT INTO operation_history (operation_json, created_at_ms, applied) VALUES (?1, ?2, 1)", params![json, now])
            .map_err(|error| format!("Could not record classification history: {error}"))?;
    }
    Ok(())
}

fn classification_destination(
    root: &Path,
    connection: &Connection,
    current: &str,
    original: Option<&str>,
    class_id: Option<&str>,
    image_id: &str,
) -> Result<String, String> {
    let preferred = if let Some(class_id) = class_id {
        let name: String = connection
            .query_row(
                "SELECT name FROM classes WHERE id = ?1",
                [class_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Could not find classification class: {error}"))?;
        format!(
            "{}/{}",
            class_directory_name(&name)?,
            file_name(original.unwrap_or(current))
        )
    } else {
        original.unwrap_or(current).to_owned()
    };
    if preferred == current || !relative_path(root, &preferred)?.exists() {
        return Ok(preferred);
    }
    let path = Path::new(&preferred);
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str());
    let short_id = &image_id[..image_id.len().min(8)];
    for index in 0..10_000 {
        let suffix = if index == 0 {
            format!("-{short_id}")
        } else {
            format!("-{short_id}-{index}")
        };
        let name = match extension {
            Some(ext) => format!("{stem}{suffix}.{ext}"),
            None => format!("{stem}{suffix}"),
        };
        let candidate = normalize_path(&parent.join(name));
        if !relative_path(root, &candidate)?.exists() {
            return Ok(candidate);
        }
    }
    Err("Could not find an available file name in the class directory.".to_owned())
}

fn class_directory_name(name: &str) -> Result<String, String> {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\' | ':') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim().trim_matches('.').trim().to_owned();
    if sanitized.is_empty()
        || sanitized.eq_ignore_ascii_case("deleted")
        || sanitized == project::PROJECT_DIRECTORY
        || sanitized == project::LEGACY_PROJECT_DIRECTORY
    {
        return Err(format!(
            "'{name}' cannot be used as a class directory name."
        ));
    }
    Ok(sanitized)
}

fn move_path(root: &Path, from: &str, to: &str) -> Result<(), String> {
    let source = relative_path(root, from)?;
    let destination = relative_path(root, to)?;
    if !source.is_file() {
        return Err(format!(
            "The source image does not exist: {}",
            source.display()
        ));
    }
    if destination.exists() {
        return Err(format!(
            "The class destination already exists: {}",
            destination.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Could not create class directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::rename(&source, &destination).map_err(|error| {
        format!(
            "Could not move {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn relative_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("A classification path is invalid.".to_owned());
    }
    Ok(root.join(path))
}

fn require_classification(connection: &Connection) -> Result<String, String> {
    let record: (String, Option<String>) = connection
        .query_row(
            "SELECT task_type, classification_mode FROM project LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("Could not read classification project settings: {error}"))?;
    if record.0 != "classification" {
        return Err("This operation is only available in a classification project.".to_owned());
    }
    Ok(record.1.unwrap_or_else(|| "single".to_owned()))
}

fn assignment_rows(connection: &Connection) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut statement = connection.prepare("SELECT i.relative_path, c.name FROM images i JOIN image_classes ic ON ic.image_id = i.id JOIN classes c ON c.id = ic.class_id WHERE i.status = 'active' ORDER BY i.sort_order, c.position")
        .map_err(|error| format!("Could not prepare classification export: {error}"))?;
    let records = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Could not read classification export: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read a classification row: {error}"))?;
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    for (path, name) in records {
        if let Some(last) = rows.last_mut().filter(|item| item.0 == path) {
            last.1.push(name);
        } else {
            rows.push((path, vec![name]));
        }
    }
    Ok(rows)
}

struct ImageLookup {
    exact: HashMap<String, String>,
    names: HashMap<String, Vec<String>>,
}

fn image_lookup(connection: &Connection) -> Result<ImageLookup, String> {
    let mut statement = connection
        .prepare("SELECT id, relative_path, file_name FROM images WHERE status = 'active'")
        .map_err(|error| format!("Could not prepare image matching: {error}"))?;
    let records = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Could not load images for matching: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read an image for matching: {error}"))?;
    let mut exact = HashMap::new();
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    for (id, path, name) in records {
        exact.insert(path.to_lowercase(), id.clone());
        names.entry(name.to_lowercase()).or_default().push(id);
    }
    Ok(ImageLookup { exact, names })
}

fn resolve_image(lookup: &ImageLookup, path: &str) -> Option<String> {
    let normalized = path
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_lowercase();
    if let Some(id) = lookup.exact.get(&normalized) {
        return Some(id.clone());
    }
    let name = Path::new(path)
        .file_name()?
        .to_string_lossy()
        .to_lowercase();
    let matches = lookup.names.get(&name)?;
    (matches.len() == 1).then(|| matches[0].clone())
}

fn ensure_classes(root_path: &str, names: &[&str]) -> Result<Vec<String>, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    let mut existing = class_records(&connection)?
        .into_iter()
        .map(|item| (item.name.to_lowercase(), item.id))
        .collect::<HashMap<_, _>>();
    drop(connection);
    let mut ids = Vec::new();
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        let id = match existing.get(&key) {
            Some(id) => id.clone(),
            None => {
                let created = detection::create_class(root_path, trimmed)?;
                existing.insert(key, created.id.clone());
                created.id
            }
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                row.push(std::mem::take(&mut field));
            }
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            other => field.push(other),
        }
    }
    if quoted {
        return Err("The classification CSV has an unterminated quoted field.".to_owned());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn available_external_path(directory: &Path, file_name: &str, image_id: &str) -> PathBuf {
    let preferred = directory.join(file_name);
    if !preferred.exists() {
        return preferred;
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str());
    let suffix = &image_id[..image_id.len().min(8)];
    let name = match extension {
        Some(ext) => format!("{stem}-{suffix}.{ext}"),
        None => format!("{stem}-{suffix}"),
    };
    directory.join(name)
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .filter_map(|part| match part {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    fn classification_project(mode: &str) -> (TempDir, String, String) {
        let directory = TempDir::new().expect("temporary directory");
        ImageBuffer::from_pixel(20, 12, Rgb([20_u8, 30_u8, 40_u8]))
            .save(directory.path().join("bird, one.jpg"))
            .expect("image");
        let root = directory.path().to_string_lossy().into_owned();
        project::create(&root, "Classification", "classification", Some(mode)).expect("project");
        let image_id = project::list_images(&root, 10, 0).expect("images")[0]
            .id
            .clone();
        (directory, root, image_id)
    }

    #[test]
    fn single_label_moves_and_history_restores_the_image() {
        let (directory, root, image_id) = classification_project("single");
        let class = detection::create_class(&root, "Birds").expect("class");
        set_image_classes(&root, &image_id, vec![class.id.clone()]).expect("classify");
        assert!(directory.path().join("Birds/bird, one.jpg").is_file());
        let connection = project::connection_for_root(directory.path()).expect("database");
        assert_eq!(
            image_class_ids(&connection, &image_id).expect("labels"),
            vec![class.id]
        );
        drop(connection);
        assert_eq!(
            crate::workspace::undo(&root).expect("undo"),
            Some(image_id.clone())
        );
        assert!(directory.path().join("bird, one.jpg").is_file());
        assert_eq!(
            crate::workspace::redo(&root).expect("redo"),
            Some(image_id.clone())
        );
        assert!(directory.path().join("Birds/bird, one.jpg").is_file());
    }

    #[test]
    fn multi_label_stays_in_place_and_csv_quotes_paths() {
        let (directory, root, image_id) = classification_project("multi");
        let bird = detection::create_class(&root, "Bird").expect("bird");
        let sky = detection::create_class(&root, "Blue sky").expect("sky");
        set_image_classes(&root, &image_id, vec![sky.id, bird.id]).expect("classify");
        assert!(directory.path().join("bird, one.jpg").is_file());
        let csv = directory.path().join("labels.csv");
        export_csv(&root, &csv.to_string_lossy()).expect("export");
        let text = fs::read_to_string(csv).expect("csv");
        assert!(text.contains("\"bird, one.jpg\",Bird,Blue sky"));
        assert_eq!(parse_csv(&text).expect("parse")[1][0], "bird, one.jpg");
    }

    #[test]
    fn csv_import_matches_a_quoted_path_and_creates_classes() {
        let (directory, root, image_id) = classification_project("multi");
        let csv = directory.path().join("incoming.csv");
        fs::write(
            &csv,
            "relative_path,class_name_1,class_name_2\n\"bird, one.jpg\",Bird,Cloudy\n",
        )
        .expect("csv");
        let report = import_csv(&root, &csv.to_string_lossy()).expect("import");
        assert_eq!(report.matched_images, 1);
        let loaded = state(&root, Some(&image_id)).expect("state");
        assert_eq!(loaded.image_class_ids.len(), 2);
        assert_eq!(loaded.classes.len(), 2);
    }

    #[test]
    fn duplicate_destination_names_get_a_stable_suffix() {
        let (directory, root, image_id) = classification_project("single");
        fs::create_dir_all(directory.path().join("Birds")).expect("folder");
        fs::write(directory.path().join("Birds/bird, one.jpg"), b"occupied").expect("conflict");
        let class = detection::create_class(&root, "Birds").expect("class");
        set_image_classes(&root, &image_id, vec![class.id]).expect("classify");
        let short = &image_id[..8];
        assert!(directory
            .path()
            .join(format!("Birds/bird, one-{short}.jpg"))
            .is_file());
    }

    #[test]
    fn imports_an_existing_class_directory_layout_without_moving_files() {
        let directory = TempDir::new().expect("temporary directory");
        fs::create_dir_all(directory.path().join("Otters")).expect("class folder");
        ImageBuffer::from_pixel(18, 10, Rgb([50_u8, 60_u8, 70_u8]))
            .save(directory.path().join("Otters/swimming.png"))
            .expect("image");
        let root = directory.path().to_string_lossy().into_owned();
        project::create(&root, "Folders", "classification", Some("single")).expect("project");
        let image_id = project::list_images(&root, 10, 0).expect("images")[0]
            .id
            .clone();
        let report = import_class_directories(&root, &root).expect("import folders");
        assert_eq!(report.matched_images, 1);
        assert!(directory.path().join("Otters/swimming.png").is_file());
        let loaded = state(&root, Some(&image_id)).expect("state");
        assert_eq!(loaded.image_class_ids.len(), 1);
        assert_eq!(loaded.classes[0].name, "Otters");
    }

    #[test]
    fn reopen_finishes_a_classification_move_interrupted_after_the_filesystem_step() {
        let (directory, root, image_id) = classification_project("single");
        let class = detection::create_class(&root, "Birds").expect("class");
        let operation = ClassificationOperation {
            operation_type: "classification".to_owned(),
            image_id: image_id.clone(),
            before_class_ids: vec![],
            after_class_ids: vec![class.id.clone()],
            from_relative_path: "bird, one.jpg".to_owned(),
            to_relative_path: "Birds/bird, one.jpg".to_owned(),
            original_relative_path: Some("bird, one.jpg".to_owned()),
        };
        let pending = PendingClassificationMove {
            id: Uuid::new_v4().to_string(),
            operation_json: serde_json::to_string(&operation).expect("operation"),
            operation,
            target_class_ids: vec![class.id.clone()],
            history_sequence: None,
            desired_applied: None,
        };
        let connection = project::connection_for_root(directory.path()).expect("database");
        insert_pending(&connection, &pending).expect("journal");
        move_path(
            directory.path(),
            &pending.operation.from_relative_path,
            &pending.operation.to_relative_path,
        )
        .expect("filesystem move");
        drop(connection);

        project::open(&root).expect("recovery on reopen");
        assert!(directory.path().join("Birds/bird, one.jpg").is_file());
        assert_eq!(
            state(&root, Some(&image_id))
                .expect("state")
                .image_class_ids,
            vec![class.id]
        );
        assert!(
            crate::workspace::history_state(&root)
                .expect("history")
                .can_undo
        );
    }

    #[test]
    fn malformed_csv_is_reported_without_changing_labels() {
        let (directory, root, image_id) = classification_project("multi");
        let csv = directory.path().join("broken.csv");
        fs::write(&csv, "relative_path,class_name\n\"bird, one.jpg,Bird\n").expect("csv");
        let error = import_csv(&root, &csv.to_string_lossy()).expect_err("malformed CSV");
        assert!(error.contains("unterminated quoted field"));
        assert!(state(&root, Some(&image_id))
            .expect("state")
            .image_class_ids
            .is_empty());
    }
}
