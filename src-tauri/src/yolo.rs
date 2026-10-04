use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn prepare_export<'a>(
    directory: &Path,
    image_paths: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    // Validate every path before writing labels.txt or any annotation files.
    // macOS commonly uses a case-insensitive filesystem.
    let mut targets = HashMap::from([("labels.txt".to_owned(), "the class list".to_owned())]);
    for relative in image_paths {
        let mut target = PathBuf::from(relative);
        target.set_extension("txt");
        let key = target.to_string_lossy().to_lowercase();
        if let Some(previous) = targets.insert(key, relative.to_owned()) {
            return Err(format!(
                "YOLO label file {} conflicts between {previous} and {relative}. Rename the images or use COCO export.",
                target.display()
            ));
        }
    }
    if directory.exists() {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| format!("Could not inspect the YOLO export folder: {error}"))?;
        if entries
            .next()
            .transpose()
            .map_err(|error| format!("Could not inspect the YOLO export folder: {error}"))?
            .is_some()
        {
            return Err("Choose a new or empty folder for YOLO export to avoid overwriting files or keeping stale labels.".to_owned());
        }
    }
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create the YOLO export folder: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rejects_colliding_paths_before_creating_any_output() {
        let directory = TempDir::new().expect("temp");
        let destination = directory.path().join("export");
        for names in [
            vec!["frame.jpg", "frame.png"],
            vec!["Frame.jpg", "frame.png"],
            vec!["labels.jpg"],
        ] {
            let error = prepare_export(&destination, names.into_iter()).expect_err("collision");
            assert!(error.contains("conflicts"));
            assert!(!destination.exists());
        }
    }

    #[test]
    fn rejects_nonempty_exports_without_changing_existing_files() {
        let directory = TempDir::new().expect("temp");
        let label = directory.path().join("frame.txt");
        fs::write(&label, "previous annotations").expect("label");
        let error =
            prepare_export(directory.path(), ["frame.jpg"].into_iter()).expect_err("nonempty");
        assert!(error.contains("new or empty"));
        assert_eq!(
            fs::read_to_string(label).expect("unchanged"),
            "previous annotations"
        );
        assert!(!directory.path().join("labels.txt").exists());
    }

    #[test]
    fn allows_matching_stems_in_different_directories() {
        let directory = TempDir::new().expect("temp");
        prepare_export(
            directory.path(),
            ["a/frame.jpg", "b/frame.png", "a/labels.jpg"].into_iter(),
        )
        .expect("distinct targets");
    }
}
