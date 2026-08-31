use std::path::{Component, Path};

pub(crate) fn validate_temp_dir_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err("must be an absolute path");
    }

    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth = depth.saturating_add(1),
            Component::ParentDir => depth = depth.saturating_sub(1),
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
        }
    }
    if depth == 0 {
        return Err("must not resolve to the filesystem root");
    }

    Ok(())
}

pub(crate) fn prepare_temp_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    if path.canonicalize()? == Path::new("/") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "report temporary directory resolves to the filesystem root",
        ));
    }

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && is_report_artifact(&entry.path()) {
            std::fs::remove_file(entry.path())?;
        }
    }

    Ok(())
}

fn is_report_artifact(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("csv" | "xlsx")
    ) && path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| uuid::Uuid::parse_str(stem).is_ok())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn refuses_paths_that_lexically_resolve_to_root() {
        for path in ["/", "/app/..", "/app/reports/../.."] {
            assert!(
                validate_temp_dir_path(Path::new(path)).is_err(),
                "should reject: {path}"
            );
        }
    }

    #[test]
    fn removes_orphans_without_removing_the_dedicated_directory() {
        let temp_dir = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("fixture directory must create: {error}"));
        let path = temp_dir.path().to_path_buf();
        let orphan = path.join(format!("{}.csv", Uuid::new_v4()));
        let unrelated = path.join("keep.txt");
        std::fs::write(&orphan, b"report")
            .unwrap_or_else(|error| panic!("fixture file must write: {error}"));
        std::fs::write(&unrelated, b"unrelated")
            .unwrap_or_else(|error| panic!("unrelated fixture must write: {error}"));

        prepare_temp_dir(&path)
            .unwrap_or_else(|error| panic!("temporary directory must prepare: {error}"));

        assert!(path.is_dir());
        assert!(!orphan.exists());
        assert!(unrelated.exists());
        std::fs::remove_file(unrelated)
            .unwrap_or_else(|error| panic!("unrelated fixture must remove: {error}"));
    }
}
