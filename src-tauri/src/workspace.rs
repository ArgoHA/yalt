use crate::project::{self, ProjectSummary};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewportSnapshot {
    pub scale: f64,
    pub center_x: f64,
    pub center_y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageMove {
    operation_type: String,
    image_id: String,
    from_relative_path: String,
    to_relative_path: String,
    from_status: String,
    to_status: String,
}

struct PendingMove {
    id: String,
    image_id: String,
    from_relative_path: String,
    to_relative_path: String,
    target_status: String,
    operation_json: String,
    history_sequence: Option<i64>,
    desired_applied: Option<bool>,
}

pub fn save_viewport(
    root_path: &str,
    image_id: &str,
    viewport: Option<ViewportSnapshot>,
) -> Result<(), String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM images WHERE id = ?1)",
            [image_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not find the current image: {error}"))?;
    if !exists {
        return Err("The current image is no longer in this project.".to_owned());
    }

    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start workspace autosave: {error}"))?;
    transaction
        .execute(
            "UPDATE project SET last_image_id = ?1, updated_at_ms = ?2",
            params![image_id, now],
        )
        .map_err(|error| format!("Could not save the current image: {error}"))?;
    if let Some(viewport) = viewport {
        if !viewport.scale.is_finite()
            || viewport.scale <= 0.0
            || !viewport.center_x.is_finite()
            || !viewport.center_y.is_finite()
        {
            return Err("The viewport contains invalid coordinates.".to_owned());
        }
        transaction
            .execute(
                "INSERT INTO image_viewports (image_id, scale, center_x, center_y, updated_at_ms)\n                 VALUES (?1, ?2, ?3, ?4, ?5)\n                 ON CONFLICT(image_id) DO UPDATE SET\n                   scale = excluded.scale, center_x = excluded.center_x,\n                   center_y = excluded.center_y, updated_at_ms = excluded.updated_at_ms",
                params![image_id, viewport.scale, viewport.center_x, viewport.center_y, now],
            )
            .map_err(|error| format!("Could not save the viewport: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Could not finish workspace autosave: {error}"))
}

pub fn load_viewport(root_path: &str, image_id: &str) -> Result<Option<ViewportSnapshot>, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    connection
        .query_row(
            "SELECT scale, center_x, center_y FROM image_viewports WHERE image_id = ?1",
            [image_id],
            |row| {
                Ok(ViewportSnapshot {
                    scale: row.get(0)?,
                    center_x: row.get(1)?,
                    center_y: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Could not restore the viewport: {error}"))
}

pub fn delete_image(root_path: &str, image_id: &str) -> Result<ProjectSummary, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    let relative_path: String = connection
        .query_row(
            "SELECT relative_path FROM images WHERE id = ?1 AND status = 'active'",
            [image_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not find the image to delete: {error}"))?
        .ok_or_else(|| "Only an available image can be moved to deleted.".to_owned())?;
    let deleted_path = available_deleted_path(&root, &relative_path, image_id)?;
    let operation = ImageMove {
        operation_type: "move_image".to_owned(),
        image_id: image_id.to_owned(),
        from_relative_path: relative_path,
        to_relative_path: deleted_path,
        from_status: "active".to_owned(),
        to_status: "deleted".to_owned(),
    };
    perform_new_move(&root, &mut connection, &operation)?;
    project::project_summary(&connection)
}

pub fn restore_image(root_path: &str, image_id: &str) -> Result<ProjectSummary, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    let paths: Option<(String, String)> = connection
        .query_row(
            "SELECT relative_path, deleted_relative_path FROM images\n             WHERE id = ?1 AND status = 'deleted' AND deleted_relative_path IS NOT NULL",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Could not find the image to restore: {error}"))?;
    let (original_path, deleted_path) =
        paths.ok_or_else(|| "Only a deleted image can be restored.".to_owned())?;
    let operation = ImageMove {
        operation_type: "move_image".to_owned(),
        image_id: image_id.to_owned(),
        from_relative_path: deleted_path,
        to_relative_path: original_path,
        from_status: "deleted".to_owned(),
        to_status: "active".to_owned(),
    };
    perform_new_move(&root, &mut connection, &operation)?;
    project::project_summary(&connection)
}

pub fn undo(root_path: &str) -> Result<Option<String>, String> {
    apply_history(root_path, true)
}

pub fn redo(root_path: &str) -> Result<Option<String>, String> {
    apply_history(root_path, false)
}

pub fn history_state(root_path: &str) -> Result<HistoryState, String> {
    let root = project::dataset_root(root_path)?;
    let connection = project::connection_for_root(&root)?;
    let can_undo: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_history WHERE applied = 1)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not inspect undo history: {error}"))?;
    let can_redo: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_history WHERE applied = 0)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not inspect redo history: {error}"))?;
    Ok(HistoryState { can_undo, can_redo })
}

pub fn recover_pending_file_operations(
    root: &Path,
    connection: &mut Connection,
) -> Result<(), String> {
    let pending = {
        let mut statement = connection
            .prepare(
                "SELECT id, image_id, from_relative_path, to_relative_path, target_status,\n                        operation_json, history_sequence, desired_applied\n                 FROM file_operations ORDER BY created_at_ms",
            )
            .map_err(|error| format!("Could not inspect interrupted file operations: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(PendingMove {
                    id: row.get(0)?,
                    image_id: row.get(1)?,
                    from_relative_path: row.get(2)?,
                    to_relative_path: row.get(3)?,
                    target_status: row.get(4)?,
                    operation_json: row.get(5)?,
                    history_sequence: row.get(6)?,
                    desired_applied: row.get::<_, Option<i64>>(7)?.map(|value| value != 0),
                })
            })
            .map_err(|error| {
                format!("Could not inspect interrupted file operation rows: {error}")
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Could not read an interrupted file operation: {error}"))?
    };

    for item in pending {
        let source = relative_path(root, &item.from_relative_path)?;
        let destination = relative_path(root, &item.to_relative_path)?;
        match (source.exists(), destination.exists()) {
            (true, false) => {
                connection
                    .execute("DELETE FROM file_operations WHERE id = ?1", [&item.id])
                    .map_err(|error| {
                        format!("Could not clear an unstarted file operation: {error}")
                    })?;
            }
            (false, true) => finalize_pending_move(connection, &item)?,
            (true, true) => {
                return Err(format!(
                    "Recovery stopped because both {} and {} exist.",
                    source.display(),
                    destination.display()
                ));
            }
            (false, false) => {
                return Err(format!(
                    "Recovery stopped because neither {} nor {} exists.",
                    source.display(),
                    destination.display()
                ));
            }
        }
    }
    Ok(())
}

fn perform_new_move(
    root: &Path,
    connection: &mut Connection,
    operation: &ImageMove,
) -> Result<(), String> {
    let operation_json = serde_json::to_string(operation)
        .map_err(|error| format!("Could not record the file operation: {error}"))?;
    let pending = PendingMove {
        id: Uuid::new_v4().to_string(),
        image_id: operation.image_id.clone(),
        from_relative_path: operation.from_relative_path.clone(),
        to_relative_path: operation.to_relative_path.clone(),
        target_status: operation.to_status.clone(),
        operation_json,
        history_sequence: None,
        desired_applied: None,
    };
    insert_pending(connection, &pending)?;
    if let Err(error) = move_path(root, &pending.from_relative_path, &pending.to_relative_path) {
        let _ = connection.execute("DELETE FROM file_operations WHERE id = ?1", [&pending.id]);
        return Err(error);
    }
    finalize_pending_move(connection, &pending)
}

fn apply_history(root_path: &str, undo: bool) -> Result<Option<String>, String> {
    let root = project::dataset_root(root_path)?;
    let mut connection = project::connection_for_root(&root)?;
    let query = if undo {
        "SELECT sequence, operation_json FROM operation_history WHERE applied = 1 ORDER BY sequence DESC LIMIT 1"
    } else {
        "SELECT sequence, operation_json FROM operation_history WHERE applied = 0 ORDER BY sequence ASC LIMIT 1"
    };
    let record: Option<(i64, String)> = connection
        .query_row(query, [], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()
        .map_err(|error| format!("Could not read operation history: {error}"))?;
    let Some((sequence, json)) = record else {
        return Ok(None);
    };
    let affected_image_id = history_image_id(&json);
    if crate::detection::is_annotation_operation(&json) {
        crate::detection::apply_annotation_history(&mut connection, sequence, &json, undo)?;
        return Ok(affected_image_id);
    }
    if crate::segmentation::is_polygon_operation(&json) {
        crate::segmentation::apply_polygon_history(&mut connection, sequence, &json, undo)?;
        return Ok(affected_image_id);
    }
    if crate::classification::is_classification_operation(&json) {
        crate::classification::apply_history(&root, &mut connection, sequence, &json, undo)?;
        return Ok(affected_image_id);
    }
    let operation: ImageMove = serde_json::from_str(&json)
        .map_err(|error| format!("Could not understand operation history: {error}"))?;
    let (from, to, target_status, desired_applied) = if undo {
        (
            operation.to_relative_path.clone(),
            operation.from_relative_path.clone(),
            operation.from_status.clone(),
            false,
        )
    } else {
        (
            operation.from_relative_path.clone(),
            operation.to_relative_path.clone(),
            operation.to_status.clone(),
            true,
        )
    };
    let pending = PendingMove {
        id: Uuid::new_v4().to_string(),
        image_id: operation.image_id,
        from_relative_path: from,
        to_relative_path: to,
        target_status,
        operation_json: json,
        history_sequence: Some(sequence),
        desired_applied: Some(desired_applied),
    };
    insert_pending(&connection, &pending)?;
    if let Err(error) = move_path(
        &root,
        &pending.from_relative_path,
        &pending.to_relative_path,
    ) {
        let _ = connection.execute("DELETE FROM file_operations WHERE id = ?1", [&pending.id]);
        return Err(error);
    }
    finalize_pending_move(&mut connection, &pending)?;
    Ok(Some(pending.image_id))
}

fn history_image_id(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if let Some(id) = value
        .get("image_id")
        .or_else(|| value.get("imageId"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(id.to_owned());
    }
    for collection in ["before", "after"] {
        if let Some(id) = value
            .get(collection)
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("imageId").or_else(|| item.get("image_id")))
            .and_then(serde_json::Value::as_str)
        {
            return Some(id.to_owned());
        }
    }
    None
}

fn insert_pending(connection: &Connection, pending: &PendingMove) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO file_operations\n             (id, image_id, from_relative_path, to_relative_path, target_status, operation_json, history_sequence, desired_applied, created_at_ms)\n             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                pending.id,
                pending.image_id,
                pending.from_relative_path,
                pending.to_relative_path,
                pending.target_status,
                pending.operation_json,
                pending.history_sequence,
                pending.desired_applied,
                project::now_ms()?
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("Could not prepare the recoverable file operation: {error}"))
}

fn finalize_pending_move(connection: &mut Connection, pending: &PendingMove) -> Result<(), String> {
    let now = project::now_ms()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not finalize the file operation: {error}"))?;
    let deleted_path = if pending.target_status == "deleted" {
        Some(pending.to_relative_path.as_str())
    } else {
        None
    };
    transaction
        .execute(
            "UPDATE images SET status = ?1, deleted_relative_path = ?2, updated_at_ms = ?3 WHERE id = ?4",
            params![pending.target_status, deleted_path, now, pending.image_id],
        )
        .map_err(|error| format!("Could not update the moved image: {error}"))?;

    if let Some(sequence) = pending.history_sequence {
        transaction
            .execute(
                "UPDATE operation_history SET applied = ?1 WHERE sequence = ?2",
                params![pending.desired_applied.unwrap_or(true), sequence],
            )
            .map_err(|error| format!("Could not update operation history: {error}"))?;
    } else {
        transaction
            .execute("DELETE FROM operation_history WHERE applied = 0", [])
            .map_err(|error| format!("Could not clear obsolete redo history: {error}"))?;
        transaction
            .execute(
                "INSERT INTO operation_history (operation_json, created_at_ms, applied) VALUES (?1, ?2, 1)",
                params![pending.operation_json, now],
            )
            .map_err(|error| format!("Could not record operation history: {error}"))?;
    }
    transaction
        .execute(
            "UPDATE project SET updated_at_ms = ?1,\n               last_image_id = CASE WHEN last_image_id = ?2 AND ?3 = 'deleted' THEN NULL ELSE last_image_id END",
            params![now, pending.image_id, pending.target_status],
        )
        .map_err(|error| format!("Could not update project state after the move: {error}"))?;
    transaction
        .execute("DELETE FROM file_operations WHERE id = ?1", [&pending.id])
        .map_err(|error| format!("Could not complete the file operation journal: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit the moved image: {error}"))
}

fn move_path(root: &Path, from_relative: &str, to_relative: &str) -> Result<(), String> {
    let source = relative_path(root, from_relative)?;
    let destination = relative_path(root, to_relative)?;
    if !source.is_file() {
        return Err(format!(
            "The source image does not exist: {}",
            source.display()
        ));
    }
    if destination.exists() {
        return Err(format!(
            "The destination already exists: {}",
            destination.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Could not create the destination directory {}: {error}",
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

fn available_deleted_path(root: &Path, original: &str, image_id: &str) -> Result<String, String> {
    let preferred = format!("deleted/{original}");
    if !relative_path(root, &preferred)?.exists() {
        return Ok(preferred);
    }
    let path = Path::new(original);
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str());
    let suffix = &image_id[..image_id.len().min(8)];
    let file_name = match extension {
        Some(extension) => format!("{stem}-deleted-{suffix}.{extension}"),
        None => format!("{stem}-deleted-{suffix}"),
    };
    Ok(format!(
        "deleted/{}",
        parent.join(file_name).to_string_lossy()
    ))
}

fn relative_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("A project-relative path is invalid.".to_owned());
    }
    Ok(root.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    fn project_with_image() -> (TempDir, ProjectSummary, String) {
        let directory = TempDir::new().expect("temporary directory");
        let image_path = directory.path().join("frame.jpg");
        ImageBuffer::from_pixel(64, 48, Rgb([40_u8, 50_u8, 60_u8]))
            .save(&image_path)
            .expect("test image");
        let root = directory.path().to_string_lossy().into_owned();
        let summary = project::create(&root, "Delete test", "detection", None).expect("project");
        let image_id = project::list_images(&root, 10, 0).expect("images")[0]
            .id
            .clone();
        (directory, summary, image_id)
    }

    #[test]
    fn delete_undo_redo_and_restore_are_recoverable() {
        let (directory, _, image_id) = project_with_image();
        let root = directory.path().to_string_lossy();
        let deleted = delete_image(&root, &image_id).expect("delete");
        assert_eq!(deleted.image_count, 0);
        assert_eq!(deleted.deleted_image_count, 1);
        assert!(!directory.path().join("frame.jpg").exists());
        assert!(directory.path().join("deleted/frame.jpg").exists());
        assert!(history_state(&root).expect("history").can_undo);

        assert_eq!(undo(&root).expect("undo"), Some(image_id.clone()));
        assert!(directory.path().join("frame.jpg").exists());
        assert!(history_state(&root).expect("history").can_redo);

        assert_eq!(redo(&root).expect("redo"), Some(image_id.clone()));
        assert!(directory.path().join("deleted/frame.jpg").exists());

        let restored = restore_image(&root, &image_id).expect("restore");
        assert_eq!(restored.image_count, 1);
        assert!(directory.path().join("frame.jpg").exists());
    }

    #[test]
    fn viewport_round_trips() {
        let (directory, _, image_id) = project_with_image();
        let root = directory.path().to_string_lossy();
        let viewport = ViewportSnapshot {
            scale: 1.75,
            center_x: 31.0,
            center_y: 22.5,
        };
        save_viewport(&root, &image_id, Some(viewport.clone())).expect("save viewport");
        assert_eq!(
            load_viewport(&root, &image_id).expect("load viewport"),
            Some(viewport)
        );
    }

    #[test]
    fn reopen_finishes_a_move_interrupted_after_the_filesystem_step() {
        let (directory, _, image_id) = project_with_image();
        let root = directory.path().to_string_lossy().into_owned();
        let operation = ImageMove {
            operation_type: "move_image".to_owned(),
            image_id: image_id.clone(),
            from_relative_path: "frame.jpg".to_owned(),
            to_relative_path: "deleted/frame.jpg".to_owned(),
            from_status: "active".to_owned(),
            to_status: "deleted".to_owned(),
        };
        let pending = PendingMove {
            id: Uuid::new_v4().to_string(),
            image_id: image_id.clone(),
            from_relative_path: operation.from_relative_path.clone(),
            to_relative_path: operation.to_relative_path.clone(),
            target_status: operation.to_status.clone(),
            operation_json: serde_json::to_string(&operation).expect("operation"),
            history_sequence: None,
            desired_applied: None,
        };
        let connection = project::connection_for_root(directory.path()).expect("database");
        insert_pending(&connection, &pending).expect("journal");
        move_path(
            directory.path(),
            &pending.from_relative_path,
            &pending.to_relative_path,
        )
        .expect("filesystem move");
        drop(connection);

        let reopened = project::open(&root).expect("recovery on reopen");
        assert_eq!(reopened.deleted_image_count, 1);
        assert!(directory.path().join("deleted/frame.jpg").is_file());
        assert_eq!(
            project::list_images(&root, 10, 0).expect("images")[0].status,
            "deleted"
        );
        assert!(history_state(&root).expect("history").can_undo);
    }
}
