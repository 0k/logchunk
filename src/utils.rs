use std::env;
use std::path::{Path, PathBuf};

pub fn normalize_path(path: &Path) -> Result<PathBuf, String> {
    let mut parts: Vec<String> = Vec::new();

    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        // If the path is relative, we need to prepend the current directory
        let current_dir = env::current_dir().map_err(|e| e.to_string())?;
        current_dir.join(path)
    };
    for component in path.components() {
        match component {
            std::path::Component::Normal(c) => parts.push(c.to_string_lossy().to_string()),
            std::path::Component::ParentDir => {
                if !parts.is_empty() {
                    parts.pop();
                }
            }
            std::path::Component::CurDir => {} // Ignore "."
            std::path::Component::RootDir => parts.push("/".to_string()),
            std::path::Component::Prefix(_) => {
                return Err("Prefix paths are not supported".to_string())
            }
        }
    }

    // Construct the absolute path
    let mut absolute_path = PathBuf::from("/");
    for part in parts {
        absolute_path.push(part);
    }

    Ok(absolute_path)
}

#[cfg(test)]
mod tests {

    use crate::utils::normalize_path;
    use std::env;
    use std::io;
    use std::path::Path;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // Define a struct that holds the temporary directory
    struct ScratchDir {
        dir: TempDir,
    }

    // Implement the Drop trait for ScratchDir
    impl Drop for ScratchDir {
        fn drop(&mut self) {
            // TempDir will automatically clean up its directory when dropped
        }
    }

    // Create a new temporary directory
    impl ScratchDir {
        fn new() -> io::Result<Self> {
            Ok(ScratchDir {
                dir: TempDir::new()?,
            })
        }

        // Get a reference to the temporary directory's path
        fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    #[test]
    fn test_normalize_absolute_path() {
        let path = Path::new("/foo/bar");
        let result = normalize_path(&path);
        assert_eq!(result, Ok(PathBuf::from("/foo/bar")));

        let path = Path::new("/foo/../bar");
        let result = normalize_path(&path);
        assert_eq!(result, Ok(PathBuf::from("/bar")));

        let path = Path::new("/foo/./bar");
        let result = normalize_path(&path);
        assert_eq!(result, Ok(PathBuf::from("/foo/bar")));

        let path = Path::new("/foo/./bar/..");
        let result = normalize_path(&path);
        assert_eq!(result, Ok(PathBuf::from("/foo")));

        let path = Path::new("/foo/./bar/../..");
        let result = normalize_path(&path);
        assert_eq!(result, Ok(PathBuf::from("/")));

        let path = Path::new("/foo/./bar/../../..");
        let result = normalize_path(&path);
        assert_eq!(result, Ok(PathBuf::from("/")));
    }

    #[test]
    fn test_normalize_relative_path() {
        let dir = ScratchDir::new().unwrap();

        // move cwd to the temporary directory
        env::set_current_dir(dir.path()).unwrap();

        let path = Path::new("foo/bar");
        let result = normalize_path(&path);
        assert_eq!(result, Ok(dir.path().join(PathBuf::from("foo/bar"))));
    }

    #[test]
    fn test_normalize_link_path() {
        let dir = ScratchDir::new().unwrap();

        // move cwd to the temporary directory
        env::set_current_dir(dir.path()).unwrap();

        // Create a temp directory foo
        let foo = dir.path().join("foo");
        std::fs::create_dir(&foo).unwrap();
        // Create a symlink bar to foo
        let bar = dir.path().join("bar");
        std::os::unix::fs::symlink("foo", &bar).unwrap();

        let path = Path::new("bar");
        let result = normalize_path(&path);

        // We don't expect the symlink to be resolved
        assert_eq!(result, Ok(dir.path().join(PathBuf::from("bar"))));
    }
}
