//! Read bounded regular files and parse the leading module declaration header.
use super::{bundle::CapturedSource, names};
use crate::error::{ErrorCode, KoruError, Result};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

pub(super) fn budget(kind: &str) -> KoruError {
    KoruError::new(ErrorCode::BudgetExhausted, format!("{kind} limit exceeded"))
}
pub(super) fn read_source(path: &Path, limit: usize) -> Result<CapturedSource> {
    // Component checks reject static symlink escapes. A hostile concurrent
    // filesystem writer needs handle-relative reads in a later hardened loader.
    for ancestor in path.ancestors().take_while(|part| *part != Path::new("/")) {
        if !ancestor.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|error| KoruError::at_path(ancestor.to_owned(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("symlink source path {}", path.display()),
            ));
        }
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| KoruError::at_path(path.to_owned(), error))?;
    if !metadata.file_type().is_file() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("source {} must be a regular file", path.display()),
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(budget("source file bytes"));
    }
    let file = File::open(path).map_err(|error| KoruError::at_path(path.to_owned(), error))?;
    let mut bytes = Vec::new();
    file.take((limit as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| KoruError::at_path(path.to_owned(), error))?;
    if bytes.len() > limit {
        return Err(budget("source file bytes"));
    }
    if bytes.starts_with(b"\x1bLua") {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "Lua bytecode is not supported",
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        KoruError::new(
            ErrorCode::Validation,
            format!("source {} must be UTF-8", path.display()),
        )
    })?;
    let dependencies = parse_directives(text)?;
    Ok(CapturedSource {
        bytes,
        dependencies,
    })
}
fn parse_directives(text: &str) -> Result<Vec<String>> {
    let mut dependencies = Vec::new();
    let mut long_comment_close: Option<String> = None;
    for line in text.lines() {
        let mut rest = line.trim_start();
        if let Some(close) = &long_comment_close {
            let Some((_, after)) = rest.split_once(close.as_str()) else {
                continue;
            };
            rest = after.trim_start();
            long_comment_close = None;
        }
        if let Some(opener) = rest.strip_prefix("--[") {
            let equals = opener.bytes().take_while(|byte| *byte == b'=').count();
            if opener.as_bytes().get(equals) == Some(&b'[') {
                let close = format!("]{}]", "=".repeat(equals));
                let after_open = &opener[equals + 1..];
                if let Some((_, after)) = after_open.split_once(&close) {
                    rest = after.trim_start();
                } else {
                    long_comment_close = Some(close);
                    continue;
                }
            }
        }
        if let Some(name) = rest.strip_prefix("-- koru-module:") {
            let name = name.trim();
            names::module(name)?;
            if dependencies.iter().any(|existing| existing == name) {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    format!("duplicate module directive {name}"),
                ));
            }
            dependencies.push(name.to_owned());
        } else if !rest.is_empty() && !rest.starts_with("--") {
            break;
        }
    }
    Ok(dependencies)
}
