//! Files for `prism fmt`: collecting them from command-line paths (D11 of `prism-fmt`) and
//! formatting or checking one.

use crate::{format_source, FormatError};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The files `paths` name: each file as given, and every `*.prism` file under each directory,
/// recursively and in sorted order, skipping directories whose name starts with `.` and not
/// following symbolic links to directories. Fails on the first path that cannot be read.
pub fn collect_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>, (PathBuf, io::Error)> {
    let mut files = Vec::new();
    for path in paths {
        let metadata = fs::metadata(path).map_err(|err| (path.clone(), err))?;
        if metadata.is_dir() {
            walk(path, &mut files)?;
        } else {
            files.push(path.clone());
        }
    }
    Ok(files)
}

fn walk(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), (PathBuf, io::Error)> {
    let entries = fs::read_dir(directory).map_err(|err| (directory.to_path_buf(), err))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| (directory.to_path_buf(), err))?;
        paths.push(entry.path());
    }
    paths.sort();
    for path in paths {
        let metadata = fs::symlink_metadata(&path).map_err(|err| (path.clone(), err))?;
        let is_hidden = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.'));
        if metadata.is_dir() {
            if !is_hidden {
                walk(&path, files)?;
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "prism")
            && (metadata.is_file() || fs::metadata(&path).is_ok_and(|m| m.is_file()))
        {
            files.push(path);
        }
    }
    Ok(())
}

/// What formatting one file found.
#[derive(Debug)]
pub enum FileOutcome {
    /// The file is already formatted.
    Unchanged,
    /// The file is not formatted; with `write`, it now is.
    Changed,
    /// The file could not be formatted; it is untouched.
    Failed {
        /// The file's text, when it could be read, for rendering a diagnostic.
        source: Option<String>,
        error: FileError,
    },
}

/// Why one file could not be formatted.
#[derive(Debug)]
pub enum FileError {
    Io(io::Error),
    Format(FormatError),
}

/// Formats the file at `path`, rewriting it when `write` and it is not formatted. A file that
/// is already formatted is never written.
pub fn format_file(path: &Path, write: bool) -> FileOutcome {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            return FileOutcome::Failed {
                source: None,
                error: FileError::Io(err),
            }
        }
    };
    let formatted = match format_source(&source) {
        Ok(formatted) => formatted,
        Err(err) => {
            return FileOutcome::Failed {
                source: Some(source),
                error: FileError::Format(err),
            }
        }
    };
    if formatted == source {
        return FileOutcome::Unchanged;
    }
    if write {
        if let Err(err) = fs::write(path, formatted) {
            return FileOutcome::Failed {
                source: Some(source),
                error: FileError::Io(err),
            };
        }
    }
    FileOutcome::Changed
}
