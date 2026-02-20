use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub path: PathBuf,
}

pub fn list_directory<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    // Read the directory components
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let file_name = entry.file_name().to_string_lossy().to_string();

        entries.push(FileEntry {
            name: file_name,
            is_dir: metadata.is_dir(),
            path: entry.path(),
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_list_directory_normal_cases() {
        // Arrange
        let tmp_dir = tempdir().unwrap();
        let base_path = tmp_dir.path();

        // Create a few files and a directory
        fs::write(base_path.join("file1.dlt"), b"dummy").unwrap();
        fs::write(base_path.join("file2.gz"), b"dummy").unwrap();
        fs::create_dir(base_path.join("sub_dir")).unwrap();

        // Act
        let mut entries = list_directory(base_path).unwrap();

        // Assert
        // We expect it to be sorted (for consistency)
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "file1.dlt");
        assert_eq!(entries[0].is_dir, false);
        assert_eq!(entries[1].name, "file2.gz");
        assert_eq!(entries[1].is_dir, false);
        assert_eq!(entries[2].name, "sub_dir");
        assert_eq!(entries[2].is_dir, true);
    }

    #[test]
    fn test_list_directory_not_found() {
        // Arrange
        let path = Path::new("/path/that/definitely/does/not/exist");

        // Act
        let result = list_directory(path);

        // Assert
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_list_directory_empty() {
        // Arrange
        let tmp_dir = tempdir().unwrap();
        let base_path = tmp_dir.path();

        // Act
        let entries = list_directory(base_path).unwrap();

        // Assert
        assert_eq!(entries.len(), 0);
    }
}
