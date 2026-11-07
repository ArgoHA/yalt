use crate::detection::{with_txt_extension, IndexedImage};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A label file matched to an indexed image: (path, image id, width, height).
pub type MatchedLabelFile = (PathBuf, String, u32, u32);

/// Pairs every `.txt` under `directory` (except the class list) with an image,
/// by relative path first and then by a unique file stem. Returns the matches
/// and the number of label files that matched no image.
pub fn match_label_files(
    directory: &Path,
    images: &[IndexedImage],
) -> (Vec<MatchedLabelFile>, usize) {
    let mut by_relative = HashMap::new();
    let mut by_stem: HashMap<String, Vec<&IndexedImage>> = HashMap::new();
    for image in images {
        by_relative.insert(with_txt_extension(&image.1).to_lowercase(), image);
        let stem = Path::new(&image.2)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&image.2)
            .to_lowercase();
        by_stem.entry(stem).or_default().push(image);
    }
    let mut matched = Vec::new();
    let mut unmatched = 0;
    for entry in WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        let is_txt = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("txt"));
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !entry.file_type().is_file()
            || !is_txt
            || name.eq_ignore_ascii_case("labels.txt")
            || name.eq_ignore_ascii_case("classes.txt")
        {
            continue;
        }
        let relative = path
            .strip_prefix(directory)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        let image = by_relative.get(&relative).copied().or_else(|| {
            let stem = path.file_stem()?.to_string_lossy().to_lowercase();
            match by_stem.get(&stem)?.as_slice() {
                [only] => Some(*only),
                _ => None,
            }
        });
        match image {
            Some((id, _, _, width, height)) => {
                matched.push((path.to_path_buf(), id.clone(), *width, *height))
            }
            None => unmatched += 1,
        }
    }
    (matched, unmatched)
}

pub fn write_class_list<'a>(
    directory: &Path,
    names: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    fs::write(
        directory.join("labels.txt"),
        format!("{}\n", names.collect::<Vec<_>>().join("\n")),
    )
    .map_err(|error| format!("Could not write labels.txt: {error}"))
}

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
