use rusqlite::{Connection, Transaction};
use std::fs;
use std::path::Path;

pub const CURRENT_SCHEMA_VERSION: i64 = 4;

const MIGRATION_1: &str = r#"
CREATE TABLE project (
    id                  TEXT PRIMARY KEY NOT NULL,
    name                TEXT NOT NULL,
    task_type           TEXT NOT NULL CHECK(task_type IN ('detection', 'segmentation', 'classification')),
    classification_mode TEXT CHECK(classification_mode IS NULL OR classification_mode IN ('single', 'multi')),
    root_path           TEXT NOT NULL,
    created_at_ms       INTEGER NOT NULL,
    updated_at_ms       INTEGER NOT NULL,
    last_image_id       TEXT
);

CREATE TABLE classes (
    id           TEXT PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL,
    position     INTEGER NOT NULL UNIQUE,
    color        TEXT NOT NULL,
    shortcut     TEXT UNIQUE,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE images (
    id             TEXT PRIMARY KEY NOT NULL,
    relative_path  TEXT NOT NULL UNIQUE,
    file_name      TEXT NOT NULL,
    width          INTEGER NOT NULL,
    height         INTEGER NOT NULL,
    byte_size      INTEGER NOT NULL,
    modified_at_ms INTEGER NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'deleted', 'missing')),
    sort_order     INTEGER NOT NULL,
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL
);

CREATE INDEX images_status_sort_idx ON images(status, sort_order);

CREATE TABLE annotations (
    id            TEXT PRIMARY KEY NOT NULL,
    image_id      TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    class_id      TEXT NOT NULL REFERENCES classes(id),
    kind          TEXT NOT NULL CHECK(kind IN ('bbox', 'polygon')),
    geometry_json TEXT NOT NULL,
    is_visible    INTEGER NOT NULL DEFAULT 1,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX annotations_image_idx ON annotations(image_id);

CREATE TABLE image_classes (
    image_id      TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    class_id      TEXT NOT NULL REFERENCES classes(id),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(image_id, class_id)
);

CREATE TABLE drafts (
    image_id      TEXT PRIMARY KEY NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL CHECK(kind IN ('bbox', 'polygon')),
    payload_json  TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE operation_history (
    sequence      INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
"#;

const MIGRATION_2: &str = r#"
ALTER TABLE images ADD COLUMN orientation INTEGER NOT NULL DEFAULT 1;
ALTER TABLE images ADD COLUMN deleted_relative_path TEXT;
ALTER TABLE operation_history ADD COLUMN applied INTEGER NOT NULL DEFAULT 1;

CREATE TABLE image_viewports (
    image_id      TEXT PRIMARY KEY NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    scale         REAL NOT NULL,
    center_x      REAL NOT NULL,
    center_y      REAL NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE file_operations (
    id                 TEXT PRIMARY KEY NOT NULL,
    image_id           TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    from_relative_path TEXT NOT NULL,
    to_relative_path   TEXT NOT NULL,
    target_status      TEXT NOT NULL CHECK(target_status IN ('active', 'deleted')),
    operation_json     TEXT NOT NULL,
    history_sequence   INTEGER,
    desired_applied    INTEGER,
    created_at_ms      INTEGER NOT NULL
);
"#;

const MIGRATION_3: &str = r#"
CREATE UNIQUE INDEX classes_name_nocase_idx ON classes(name COLLATE NOCASE);
CREATE INDEX annotations_image_kind_idx ON annotations(image_id, kind);
"#;

const MIGRATION_4: &str = r#"
ALTER TABLE images ADD COLUMN classification_original_path TEXT;

CREATE TABLE classification_file_operations (
    id                 TEXT PRIMARY KEY NOT NULL,
    image_id           TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    from_relative_path TEXT NOT NULL,
    to_relative_path   TEXT NOT NULL,
    target_class_ids   TEXT NOT NULL,
    operation_json     TEXT NOT NULL,
    history_sequence   INTEGER,
    desired_applied    INTEGER,
    created_at_ms      INTEGER NOT NULL
);

CREATE INDEX image_classes_class_idx ON image_classes(class_id);
"#;

pub fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;\nPRAGMA journal_mode = WAL;\nPRAGMA synchronous = NORMAL;\nPRAGMA busy_timeout = 5000;",
        )
        .map_err(|error| format!("Could not configure the project database: {error}"))
}

pub fn migrate(connection: &mut Connection, database_path: &Path) -> Result<(), String> {
    let current_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("Could not read the project schema version: {error}"))?;

    if current_version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "This project uses schema version {current_version}, but this app supports up to {CURRENT_SCHEMA_VERSION}."
        ));
    }

    if current_version > 0 && current_version < CURRENT_SCHEMA_VERSION {
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|error| {
                format!("Could not checkpoint the project before migration: {error}")
            })?;
        let backup_path =
            database_path.with_extension(format!("before-v{CURRENT_SCHEMA_VERSION}.sqlite"));
        fs::copy(database_path, &backup_path).map_err(|error| {
            format!(
                "Could not back up the project before migration to {}: {error}",
                backup_path.display()
            )
        })?;
    }

    if current_version < 1 {
        apply_migration(connection, 1, MIGRATION_1)?;
    }
    if current_version < 2 {
        apply_migration(connection, 2, MIGRATION_2)?;
    }
    if current_version < 3 {
        apply_migration(connection, 3, MIGRATION_3)?;
    }
    if current_version < 4 {
        apply_migration(connection, 4, MIGRATION_4)?;
    }

    Ok(())
}

fn apply_migration(connection: &mut Connection, version: i64, sql: &str) -> Result<(), String> {
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start schema migration {version}: {error}"))?;
    run_migration(&transaction, version, sql)?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit schema migration {version}: {error}"))
}

fn run_migration(transaction: &Transaction<'_>, version: i64, sql: &str) -> Result<(), String> {
    transaction
        .execute_batch(sql)
        .map_err(|error| format!("Could not apply schema migration {version}: {error}"))?;
    transaction
        .pragma_update(None, "user_version", version)
        .map_err(|error| format!("Could not record schema migration {version}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    #[test]
    fn upgrades_a_phase_one_database_and_keeps_a_backup() {
        let directory = TempDir::new().expect("temporary directory");
        let path = directory.path().join("project.sqlite");
        let mut connection = Connection::open(&path).expect("database");
        configure_connection(&connection).expect("configure");
        apply_migration(&mut connection, 1, MIGRATION_1).expect("phase one schema");
        migrate(&mut connection, &path).expect("phase two migration");

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert!(directory.path().join("project.before-v4.sqlite").is_file());
        let viewport_table: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'image_viewports'",
                [],
                |row| row.get(0),
            )
            .expect("viewport table");
        assert_eq!(viewport_table, "image_viewports");
    }
}
