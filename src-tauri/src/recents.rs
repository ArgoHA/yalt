use crate::project::ProjectSummary;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::fs;
use std::path::Path;

const LEGACY_APP_IDENTIFIER: &str = "com.argoha.labeler";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub name: String,
    pub root_path: String,
    pub task_type: String,
    pub last_opened_at_ms: i64,
    pub available: bool,
    #[serde(default)]
    pub preview_image_id: Option<String>,
}

pub fn list(app_data_directory: &Path) -> Result<Vec<RecentProject>, String> {
    let path = app_data_directory.join("recent-projects.json");
    let source = if path.is_file() {
        path
    } else if let Some(parent) = app_data_directory.parent() {
        let legacy = parent
            .join(LEGACY_APP_IDENTIFIER)
            .join("recent-projects.json");
        if legacy.is_file() {
            legacy
        } else {
            return Ok(Vec::new());
        }
    } else {
        return Ok(Vec::new());
    };
    let contents = fs::read_to_string(&source)
        .map_err(|error| format!("Could not read recent projects: {error}"))?;
    let mut projects: Vec<RecentProject> = serde_json::from_str(&contents)
        .map_err(|error| format!("Could not parse recent projects: {error}"))?;
    for project in &mut projects {
        project.available =
            crate::project::project_database_path(Path::new(&project.root_path)).is_file();
        project.preview_image_id = if project.available {
            crate::project::first_active_image_id(&project.root_path)
        } else {
            None
        };
    }
    projects.sort_by_key(|project| Reverse(project.last_opened_at_ms));
    Ok(projects)
}

pub fn remember(app_data_directory: &Path, project: &ProjectSummary) -> Result<(), String> {
    fs::create_dir_all(app_data_directory)
        .map_err(|error| format!("Could not create app data storage: {error}"))?;
    let mut projects = list(app_data_directory).unwrap_or_default();
    projects.retain(|recent| recent.root_path != project.root_path);
    projects.insert(
        0,
        RecentProject {
            name: project.name.clone(),
            root_path: project.root_path.clone(),
            task_type: project.task_type.clone(),
            last_opened_at_ms: project.updated_at_ms,
            available: true,
            preview_image_id: crate::project::first_active_image_id(&project.root_path),
        },
    );
    projects.truncate(12);

    let path = app_data_directory.join("recent-projects.json");
    let temporary_path = app_data_directory.join("recent-projects.json.tmp");
    let json = serde_json::to_string_pretty(&projects)
        .map_err(|error| format!("Could not serialize recent projects: {error}"))?;
    fs::write(&temporary_path, json)
        .map_err(|error| format!("Could not write recent projects: {error}"))?;
    fs::rename(&temporary_path, &path)
        .map_err(|error| format!("Could not commit recent projects: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn reads_pre_release_recents_when_yalt_storage_is_empty() {
        let directory = TempDir::new().expect("temporary directory");
        let legacy = directory.path().join(LEGACY_APP_IDENTIFIER);
        fs::create_dir(&legacy).expect("legacy app directory");
        fs::write(
            legacy.join("recent-projects.json"),
            r#"[{"name":"Wildlife","rootPath":"/missing","taskType":"detection","lastOpenedAtMs":1,"available":true,"previewImageId":null}]"#,
        )
        .expect("legacy recents");

        let projects = list(&directory.path().join("com.argoha.yalt")).expect("recent projects");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Wildlife");
    }
}
