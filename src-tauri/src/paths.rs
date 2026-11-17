//! Path containment helpers shared by commands and library actions.
//!
//! Every filesystem action initiated from the UI must resolve inside the
//! configured download root. Canonicalisation is done on both sides so `..`
//! segments and symlinks cannot escape the sandbox.

use crate::error::{AppError, AppResult};
use std::path::{Component, Path, PathBuf};

pub fn canonical_root(root: &Path) -> AppResult<PathBuf> {
    root.canonicalize().map_err(|error| {
        AppError::msg(format!(
            "Download root is not accessible ({}): {error}",
            root.display()
        ))
    })
}

pub fn contained_path(root: &Path, path: &Path) -> AppResult<PathBuf> {
    let root = canonical_root(root)?;
    let candidate = path
        .canonicalize()
        .map_err(|error| AppError::msg(format!("Path is not accessible: {error}")))?;
    if !candidate.starts_with(&root) {
        return Err(AppError::msg(
            "Refusing to access a path outside the NovelForge download sandbox",
        ));
    }
    Ok(candidate)
}

/// Hidden trees such as `.novelforge-work` hold partial crawler state and must
/// never appear as library entries or count toward library bytes.
pub fn is_hidden_path(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .any(|component| match component {
            Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "novelforge-paths-{label}-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn accepts_children_and_rejects_parent_escapes() {
        let root = fixture_root("contain");
        let inside = root.join("book-id");
        std::fs::create_dir_all(&inside).unwrap();
        let outside = root
            .parent()
            .unwrap()
            .join(format!("novelforge-paths-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();

        assert_eq!(
            contained_path(&root, &inside).unwrap(),
            inside.canonicalize().unwrap()
        );
        assert!(contained_path(&root, &outside).is_err());
        assert!(contained_path(&root, &root.join("../")).is_err());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_that_escape_the_root() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("symlink");
        let outside = std::env::temp_dir().join(format!(
            "novelforge-paths-symlink-outside-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        let link = root.join("escape");
        symlink(&outside, &link).unwrap();

        assert!(contained_path(&root, &link).is_err());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn hidden_paths_are_detected_below_the_root() {
        let root = Path::new("/downloads");
        assert!(is_hidden_path(
            root,
            Path::new("/downloads/.novelforge-work/job/novels/id/artifacts/book.epub")
        ));
        assert!(is_hidden_path(
            root,
            Path::new("/downloads/.partial/book.epub")
        ));
        assert!(!is_hidden_path(
            root,
            Path::new("/downloads/book-id/book.epub")
        ));
    }
}
