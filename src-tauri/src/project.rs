use crate::migrations;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use image::ImageFormat;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use walkdir::{DirEntry, WalkDir};

pub const PROJECT_DIRECTORY: &str = ".yalt";
pub const LEGACY_PROJECT_DIRECTORY: &str = ".labeler";
pub const DATABASE_FILE: &str = "project.sqlite";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub task_type: String,
    pub classification_mode: Option<String>,
    pub root_path: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub image_count: i64,
    pub missing_image_count: i64,
    pub deleted_image_count: i64,
    pub last_image_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRecord {
    pub id: String,
    pub relative_path: String,
    pub file_name: String,
    pub width: u32,
    pub height: u32,
    pub status: String,
    pub orientation: u16,
}

pub fn create(
    root_path: &str,
    name: &str,
    task_type: &str,
    classification_mode: Option<&str>,
) -> Result<ProjectSummary, String> {
    validate_project_options(name, task_type, classification_mode)?;
    let root = canonical_dataset_root(root_path)?;
    let project_directory = root.join(PROJECT_DIRECTORY);
    let database_path = project_directory.join(DATABASE_FILE);

    if project_database_path(&root).is_file() {
        return Err(format!(
            "A yalt project already exists in {}. Open it instead.",
            root.display()
        ));
    }

    fs::create_dir_all(project_directory.join("thumbnails")).map_err(|error| {
        format!(
            "Could not create the project directory in {}: {error}",
            root.display()
        )
    })?;

    let mut connection = open_connection(&database_path)?;
    migrations::migrate(&mut connection, &database_path)?;
    let now = now_ms()?;
    let id = Uuid::new_v4().to_string();
    let mode = if task_type == "classification" {
        Some(classification_mode.unwrap_or("single"))
    } else {
        None
    };

    connection
        .execute(
            "INSERT INTO project (id, name, task_type, classification_mode, root_path, created_at_ms, updated_at_ms)\n             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, name.trim(), task_type, mode, path_to_string(&root), now],
        )
        .map_err(|error| format!("Could not create the project record: {error}"))?;

    index_images(&root, &mut connection)?;
    summary_from_connection(&connection)
}

pub fn open(root_path: &str) -> Result<ProjectSummary, String> {
    let root = canonical_dataset_root(root_path)?;
    let database_path = project_database_path(&root);
    if !database_path.is_file() {
        return Err(format!("No yalt project was found in {}.", root.display()));
    }

    let mut connection = open_connection(&database_path)?;
    migrations::migrate(&mut connection, &database_path)?;
    crate::workspace::recover_pending_file_operations(&root, &mut connection)?;
    crate::classification::recover_pending(&root, &mut connection)?;
    verify_integrity(&connection)?;
    connection
        .execute(
            "UPDATE project SET root_path = ?1, updated_at_ms = ?2",
            params![path_to_string(&root), now_ms()?],
        )
        .map_err(|error| format!("Could not update the portable project path: {error}"))?;
    index_images(&root, &mut connection)?;
    summary_from_connection(&connection)
}

pub fn rescan(root_path: &str) -> Result<ProjectSummary, String> {
    let root = canonical_dataset_root(root_path)?;
    let database_path = project_database_path(&root);
    require_project_database(&root, &database_path)?;
    let mut connection = open_connection(&database_path)?;
    verify_integrity(&connection)?;
    index_images(&root, &mut connection)?;
    summary_from_connection(&connection)
}

pub fn summary(root_path: &str) -> Result<ProjectSummary, String> {
    let root = canonical_dataset_root(root_path)?;
    let connection = connection_for_root(&root)?;
    summary_from_connection(&connection)
}

pub fn list_images(root_path: &str, limit: u32, offset: u32) -> Result<Vec<ImageRecord>, String> {
    let root = canonical_dataset_root(root_path)?;
    let database_path = project_database_path(&root);
    require_project_database(&root, &database_path)?;
    let connection = open_connection(&database_path)?;
    let mut statement = connection
        .prepare(
            "SELECT id, relative_path, file_name, width, height, status, orientation\n             FROM images\n             ORDER BY CASE status WHEN 'active' THEN 0 WHEN 'missing' THEN 1 ELSE 2 END, sort_order\n             LIMIT ?1 OFFSET ?2",
        )
        .map_err(|error| format!("Could not prepare the image list: {error}"))?;
    let rows = statement
        .query_map(params![limit.min(1000), offset], |row| {
            Ok(ImageRecord {
                id: row.get(0)?,
                relative_path: row.get(1)?,
                file_name: row.get(2)?,
                width: row.get(3)?,
                height: row.get(4)?,
                status: row.get(5)?,
                orientation: row.get(6)?,
            })
        })
        .map_err(|error| format!("Could not read the image list: {error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read an indexed image: {error}"))
}

pub fn thumbnail_data_url(root_path: &str, image_id: &str) -> Result<String, String> {
    let root = canonical_dataset_root(root_path)?;
    let database_path = project_database_path(&root);
    require_project_database(&root, &database_path)?;
    let connection = open_connection(&database_path)?;
    let (relative_path, orientation): (String, u16) = connection
        .query_row(
            "SELECT relative_path, orientation FROM images WHERE id = ?1 AND status = 'active'",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Could not find the image: {error}"))?
        .ok_or_else(|| "The image is missing or deleted.".to_owned())?;

    let cache_path = project_directory_for_root(&root)
        .join("thumbnails")
        .join(format!("{image_id}.jpg"));
    let bytes = if cache_path.is_file() {
        fs::read(&cache_path)
            .map_err(|error| format!("Could not read the cached thumbnail: {error}"))?
    } else {
        let source = safe_image_path(&root, &relative_path)?;
        let image = image::open(&source)
            .map_err(|error| format!("Could not decode {}: {error}", source.display()))?;
        let thumbnail = apply_orientation(image, orientation).thumbnail(480, 320);
        let mut bytes = Vec::new();
        thumbnail
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Jpeg)
            .map_err(|error| format!("Could not encode the thumbnail: {error}"))?;
        fs::write(&cache_path, &bytes)
            .map_err(|error| format!("Could not cache the thumbnail: {error}"))?;
        bytes
    };

    Ok(format!("data:image/jpeg;base64,{}", BASE64.encode(bytes)))
}

pub fn image_bytes(root_path: &str, image_id: &str) -> Result<Vec<u8>, String> {
    let root = canonical_dataset_root(root_path)?;
    let database_path = project_database_path(&root);
    require_project_database(&root, &database_path)?;
    let connection = open_connection(&database_path)?;
    let (relative_path, orientation): (String, u16) = connection
        .query_row(
            "SELECT relative_path, orientation FROM images WHERE id = ?1 AND status = 'active'",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Could not find the image: {error}"))?
        .ok_or_else(|| "The image is missing or deleted.".to_owned())?;
    let source = safe_image_path(&root, &relative_path)?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if orientation == 1 && matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "webp") {
        return fs::read(&source)
            .map_err(|error| format!("Could not read {}: {error}", source.display()));
    }
    let decoded = image::open(&source)
        .map_err(|error| format!("Could not decode {}: {error}", source.display()))?;
    let normalized = apply_orientation(decoded, orientation);
    let mut bytes = Vec::new();
    let output_format = if extension == "tif" || extension == "tiff" {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg
    };
    normalized
        .write_to(&mut Cursor::new(&mut bytes), output_format)
        .map_err(|error| format!("Could not prepare the image for display: {error}"))?;
    Ok(bytes)
}

fn validate_project_options(
    name: &str,
    task_type: &str,
    classification_mode: Option<&str>,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Project name cannot be empty.".to_owned());
    }
    if !matches!(task_type, "detection" | "segmentation" | "classification") {
        return Err("Choose detection, segmentation, or classification.".to_owned());
    }
    if task_type == "classification"
        && !matches!(classification_mode.unwrap_or("single"), "single" | "multi")
    {
        return Err("Classification mode must be single or multi.".to_owned());
    }
    Ok(())
}

fn canonical_dataset_root(root_path: &str) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root_path)
        .map_err(|error| format!("Could not open {root_path}: {error}"))?;
    if !root.is_dir() {
        return Err(format!("{} is not a directory.", root.display()));
    }
    Ok(root)
}

pub(crate) fn project_directory_for_root(root: &Path) -> PathBuf {
    let current = root.join(PROJECT_DIRECTORY);
    if current.join(DATABASE_FILE).is_file() {
        return current;
    }
    let legacy = root.join(LEGACY_PROJECT_DIRECTORY);
    if legacy.join(DATABASE_FILE).is_file() {
        return legacy;
    }
    current
}

pub(crate) fn project_database_path(root: &Path) -> PathBuf {
    project_directory_for_root(root).join(DATABASE_FILE)
}

fn open_connection(database_path: &Path) -> Result<Connection, String> {
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    let connection = Connection::open(database_path)
        .map_err(|error| format!("Could not open {}: {error}", database_path.display()))?;
    migrations::configure_connection(&connection)?;
    Ok(connection)
}

fn require_project_database(root: &Path, database_path: &Path) -> Result<(), String> {
    if database_path.is_file() {
        Ok(())
    } else {
        Err(format!("No yalt project was found in {}.", root.display()))
    }
}

fn verify_integrity(connection: &Connection) -> Result<(), String> {
    let result: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| format!("Could not check project integrity: {error}"))?;
    if result != "ok" {
        return Err(format!(
            "The project database failed its integrity check: {result}"
        ));
    }
    Ok(())
}

fn index_images(root: &Path, connection: &mut Connection) -> Result<(), String> {
    let now = now_ms()?;
    let mut discovered = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_visit)
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() || !is_supported_image(entry.path()) {
            continue;
        }
        let relative_path = match entry.path().strip_prefix(root) {
            Ok(path) => normalize_relative_path(path),
            Err(_) => continue,
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let (raw_width, raw_height) = match image::image_dimensions(entry.path()) {
            Ok(dimensions) => dimensions,
            Err(_) => continue,
        };
        let orientation = image_orientation(entry.path());
        let (width, height) = if matches!(orientation, 5..=8) {
            (raw_height, raw_width)
        } else {
            (raw_width, raw_height)
        };
        let modified_at_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        discovered.push((
            relative_path,
            entry.file_name().to_string_lossy().into_owned(),
            width,
            height,
            metadata.len() as i64,
            modified_at_ms,
            orientation,
        ));
    }

    discovered.sort_by_cached_key(|item| item.0.to_lowercase());
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start image indexing: {error}"))?;
    transaction
        .execute(
            "UPDATE images SET status = 'missing', updated_at_ms = ?1 WHERE status = 'active'",
            [now],
        )
        .map_err(|error| format!("Could not prepare existing image records: {error}"))?;

    let mut previous_metadata: HashMap<String, (String, i64, i64)> = HashMap::new();
    {
        let mut statement = transaction
            .prepare("SELECT relative_path, id, byte_size, modified_at_ms FROM images")
            .map_err(|error| format!("Could not inspect existing images: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get(0)?, (row.get(1)?, row.get(2)?, row.get(3)?)))
            })
            .map_err(|error| format!("Could not inspect existing image rows: {error}"))?;
        for row in rows {
            let (path, metadata) =
                row.map_err(|error| format!("Could not read image metadata: {error}"))?;
            previous_metadata.insert(path, metadata);
        }
    }

    for (sort_order, (relative_path, file_name, width, height, size, modified, orientation)) in
        discovered.iter().enumerate()
    {
        let id = previous_metadata
            .get(relative_path)
            .map(|value| value.0.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        transaction
            .execute(
                "INSERT INTO images\n                 (id, relative_path, file_name, width, height, byte_size, modified_at_ms, status, sort_order, created_at_ms, updated_at_ms, orientation)\n                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', ?8, ?9, ?9, ?10)\n                 ON CONFLICT(relative_path) DO UPDATE SET\n                   file_name = excluded.file_name, width = excluded.width, height = excluded.height,\n                   byte_size = excluded.byte_size, modified_at_ms = excluded.modified_at_ms, orientation = excluded.orientation,\n                   status = CASE WHEN images.status = 'deleted' THEN 'deleted' ELSE 'active' END,\n                   sort_order = excluded.sort_order, updated_at_ms = excluded.updated_at_ms",
                params![id, relative_path, file_name, width, height, size, modified, sort_order as i64, now, orientation],
            )
            .map_err(|error| format!("Could not index {relative_path}: {error}"))?;

        if let Some((old_id, old_size, old_modified)) = previous_metadata.get(relative_path) {
            if old_size != size || old_modified != modified {
                let thumbnail = project_directory_for_root(root)
                    .join("thumbnails")
                    .join(format!("{old_id}.jpg"));
                if thumbnail.is_file() {
                    let _ = fs::remove_file(thumbnail);
                }
            }
        }
    }

    transaction
        .execute("UPDATE project SET updated_at_ms = ?1", [now])
        .map_err(|error| format!("Could not update the project timestamp: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Could not finish image indexing: {error}"))
}

fn should_visit(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    if name == PROJECT_DIRECTORY
        || name == LEGACY_PROJECT_DIRECTORY
        || name.eq_ignore_ascii_case("deleted")
    {
        return false;
    }
    entry.depth() == 0 || !name.starts_with('.')
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}

fn image_orientation(path: &Path) -> u16 {
    let is_jpeg = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| matches!(extension.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
        .unwrap_or(false);
    if !is_jpeg {
        return 1;
    }

    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return 1,
    };
    exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok()
        .and_then(|metadata| {
            metadata
                .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|field| field.value.get_uint(0))
        })
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| matches!(value, 1..=8))
        .unwrap_or(1)
}

fn apply_orientation(image: image::DynamicImage, orientation: u16) -> image::DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

fn normalize_relative_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn safe_image_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let candidate = root.join(relative_path);
    let canonical = fs::canonicalize(&candidate)
        .map_err(|error| format!("Could not open {}: {error}", candidate.display()))?;
    if !canonical.starts_with(root) {
        return Err("The image path points outside the project directory.".to_owned());
    }
    Ok(canonical)
}

fn summary_from_connection(connection: &Connection) -> Result<ProjectSummary, String> {
    connection
        .query_row(
            "SELECT p.id, p.name, p.task_type, p.classification_mode, p.root_path,\n                    p.created_at_ms, p.updated_at_ms,\n                    SUM(CASE WHEN i.status = 'active' THEN 1 ELSE 0 END),\n                    SUM(CASE WHEN i.status = 'missing' THEN 1 ELSE 0 END),\n                    SUM(CASE WHEN i.status = 'deleted' THEN 1 ELSE 0 END),\n                    p.last_image_id\n             FROM project p LEFT JOIN images i ON 1 = 1 GROUP BY p.id",
            [],
            |row| {
                Ok(ProjectSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    task_type: row.get(2)?,
                    classification_mode: row.get(3)?,
                    root_path: row.get(4)?,
                    created_at_ms: row.get(5)?,
                    updated_at_ms: row.get(6)?,
                    image_count: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                    missing_image_count: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
                    deleted_image_count: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
                    last_image_id: row.get(10)?,
                })
            },
        )
        .map_err(|error| format!("Could not read the project: {error}"))
}

pub(crate) fn now_ms() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .map_err(|error| format!("The system clock is invalid: {error}"))
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn dataset_root(root_path: &str) -> Result<PathBuf, String> {
    canonical_dataset_root(root_path)
}

pub(crate) fn connection_for_root(root: &Path) -> Result<Connection, String> {
    let database_path = project_database_path(root);
    require_project_database(root, &database_path)?;
    open_connection(&database_path)
}

pub(crate) fn project_summary(connection: &Connection) -> Result<ProjectSummary, String> {
    summary_from_connection(connection)
}

pub(crate) fn first_active_image_id(root_path: &str) -> Option<String> {
    let root = canonical_dataset_root(root_path).ok()?;
    let connection = connection_for_root(&root).ok()?;
    connection
        .query_row(
            "SELECT id FROM images WHERE status = 'active' ORDER BY sort_order LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::time::Instant;
    use tempfile::TempDir;

    fn write_test_image(path: &Path, width: u32, height: u32) {
        let image = ImageBuffer::from_pixel(width, height, Rgb([42_u8, 55_u8, 68_u8]));
        image.save(path).expect("test image should save");
    }

    #[test]
    fn creates_indexes_and_reopens_a_project() {
        let directory = TempDir::new().expect("temporary directory");
        write_test_image(&directory.path().join("one.jpg"), 80, 60);
        fs::create_dir(directory.path().join("nested")).expect("nested directory");
        write_test_image(&directory.path().join("nested/two.png"), 20, 10);

        let root = directory.path().to_string_lossy();
        let created = create(&root, "Wildlife", "detection", None).expect("project creation");
        assert_eq!(created.name, "Wildlife");
        assert_eq!(created.image_count, 2);
        assert!(directory
            .path()
            .join(PROJECT_DIRECTORY)
            .join(DATABASE_FILE)
            .is_file());

        fs::remove_file(directory.path().join("one.jpg")).expect("remove source image");
        let reopened = open(&root).expect("project reopen");
        assert_eq!(reopened.image_count, 1);
        assert_eq!(reopened.missing_image_count, 1);
        assert_eq!(
            rusqlite::Connection::open(
                directory.path().join(PROJECT_DIRECTORY).join(DATABASE_FILE)
            )
            .expect("open database")
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("schema version"),
            migrations::CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn opens_projects_created_under_the_legacy_directory_name() {
        let directory = TempDir::new().expect("temporary directory");
        write_test_image(&directory.path().join("one.jpg"), 80, 60);
        let root = directory.path().to_string_lossy();
        create(&root, "Legacy", "detection", None).expect("project creation");
        fs::rename(
            directory.path().join(PROJECT_DIRECTORY),
            directory.path().join(LEGACY_PROJECT_DIRECTORY),
        )
        .expect("rename project directory");

        let reopened = open(&root).expect("legacy project reopen");
        assert_eq!(reopened.name, "Legacy");
        assert_eq!(reopened.image_count, 1);
    }

    #[test]
    fn skips_deleted_and_hidden_directories() {
        let directory = TempDir::new().expect("temporary directory");
        fs::create_dir(directory.path().join("deleted")).expect("deleted directory");
        fs::create_dir(directory.path().join(".hidden")).expect("hidden directory");
        write_test_image(&directory.path().join("visible.png"), 10, 10);
        write_test_image(&directory.path().join("deleted/removed.png"), 10, 10);
        write_test_image(&directory.path().join(".hidden/hidden.png"), 10, 10);

        let summary = create(
            &directory.path().to_string_lossy(),
            "Classification",
            "classification",
            Some("single"),
        )
        .expect("project creation");
        assert_eq!(summary.image_count, 1);
    }

    #[test]
    fn applies_exif_rotation_to_display_dimensions() {
        let source = image::DynamicImage::new_rgb8(120, 80);
        let rotated = apply_orientation(source, 6);
        assert_eq!((rotated.width(), rotated.height()), (80, 120));
    }

    #[test]
    fn indexes_and_decodes_webp_and_tiff_images() {
        let directory = TempDir::new().expect("temporary directory");
        write_test_image(&directory.path().join("frame.webp"), 31, 17);
        write_test_image(&directory.path().join("scan.tiff"), 19, 23);

        let root = directory.path().to_string_lossy();
        let summary = create(&root, "More formats", "detection", None).expect("project");
        assert_eq!(summary.image_count, 2);
        let images = list_images(&root, 10, 0).expect("images");
        let webp = images
            .iter()
            .find(|image| image.file_name == "frame.webp")
            .expect("webp");
        let tiff = images
            .iter()
            .find(|image| image.file_name == "scan.tiff")
            .expect("tiff");
        assert!(!image_bytes(&root, &webp.id).expect("webp bytes").is_empty());
        let tiff_bytes = image_bytes(&root, &tiff.id).expect("tiff bytes");
        assert_eq!(&tiff_bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    #[ignore = "explicit Phase 2 dataset benchmark"]
    fn indexes_ten_thousand_images_without_decoding_them_all() {
        let directory = TempDir::new().expect("temporary directory");
        let source = directory.path().join("source.png");
        write_test_image(&source, 16, 16);
        for index in 0..10_000 {
            fs::copy(
                &source,
                directory.path().join(format!("frame-{index:05}.png")),
            )
            .expect("copy benchmark image");
        }
        fs::remove_file(source).expect("remove benchmark source");

        let started = Instant::now();
        let summary = create(
            &directory.path().to_string_lossy(),
            "Ten thousand",
            "detection",
            None,
        )
        .expect("benchmark project");
        assert_eq!(summary.image_count, 10_000);
        eprintln!("indexed 10,000 images in {:?}", started.elapsed());
    }

    #[test]
    #[ignore = "explicit Phase 2 large-image benchmark"]
    fn creates_a_cached_thumbnail_for_a_large_image() {
        let directory = TempDir::new().expect("temporary directory");
        write_test_image(&directory.path().join("large.png"), 6_000, 4_000);
        let root = directory.path().to_string_lossy();
        create(&root, "Large image", "detection", None).expect("project");
        let image_id = list_images(&root, 1, 0).expect("image list")[0].id.clone();
        let started = Instant::now();
        let data = thumbnail_data_url(&root, &image_id).expect("thumbnail");
        assert!(data.starts_with("data:image/jpeg;base64,"));
        assert!(directory
            .path()
            .join(PROJECT_DIRECTORY)
            .join("thumbnails")
            .join(format!("{image_id}.jpg"))
            .is_file());
        eprintln!(
            "decoded and cached 24 MP thumbnail in {:?}",
            started.elapsed()
        );
    }
}
