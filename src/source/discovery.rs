//! Filename-only discovery; no command source is opened or interpreted.
use super::names;
use crate::error::{ErrorCode, KoruError, Result};
use std::{fs, io::ErrorKind, path::Path};

/// Discover top-level command names without evaluating Lua.
pub fn discover(commands_dir: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(commands_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(KoruError::at_path(commands_dir.to_owned(), error)),
    };
    let mut commands = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| KoruError::at_path(commands_dir.to_owned(), error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "command filename is not UTF-8",
            ));
        };
        if name == "lib" || !name.ends_with(".lua") {
            continue;
        }
        let command_name = name.strip_suffix(".lua").expect("checked suffix");
        names::command(command_name)?;
        let kind = entry
            .file_type()
            .map_err(|error| KoruError::at_path(entry.path(), error))?;
        if !kind.is_file() {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("command {command_name:?} must be a regular file without symlinks"),
            ));
        }
        commands.push(command_name.to_owned());
    }
    commands.sort_unstable();
    Ok(commands)
}
