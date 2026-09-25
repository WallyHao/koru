//! Capture bounded staged diff data through one approved, fixed Git invocation.
use crate::{
    effects::execute_process,
    error::{ErrorCode, KoruError, Result},
    lua::ApprovalProvider,
    permissions::{Broker, Environment, Policy, PreparedAction},
    runtime::ExecutionContext,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

#[cfg(target_os = "linux")]
use std::{fs, os::unix::fs::PermissionsExt};

const MAX_CHANGES: usize = 64;
const MAX_EXCERPT_BYTES: usize = 1024;
const MAX_PATH_BYTES: usize = 64 * 1024;

/// One staged file-level change, identified without exposing its path to Lua.
#[derive(Debug, Clone)]
pub(crate) struct Change {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) excerpt: String,
}

/// Bounded staged change snapshot for one plan-only workflow.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) changes: Vec<Change>,
    pub(crate) digest: [u8; 32],
}

/// Approve and capture a read-only staged diff; no Git metadata is written.
pub(crate) fn capture(
    context: &ExecutionContext,
    approval: &mut dyn ApprovalProvider,
) -> Result<Snapshot> {
    if !cfg!(target_os = "linux") {
        return Err(KoruError::new(
            ErrorCode::UnsupportedCapability,
            "Git snapshots are currently supported only on Linux",
        ));
    }
    context.ensure_active(Instant::now())?;
    let cwd = std::env::current_dir()
        .map_err(|error| KoruError::io("cannot read current directory", error))?;
    let executable = git_executable()?;
    let repository_identity = cwd.as_os_str().as_encoded_bytes().to_vec();
    let mut environment = BTreeMap::new();
    environment.insert("GIT_OPTIONAL_LOCKS".to_owned(), "0".to_owned());
    environment.insert("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned());
    environment.insert("GIT_CONFIG_GLOBAL".to_owned(), "/dev/null".to_owned());
    environment.insert("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned());
    environment.insert("GIT_NO_REPLACE_OBJECTS".to_owned(), "1".to_owned());
    let action = PreparedAction::process(
        context,
        executable,
        [
            "--no-pager",
            "diff",
            "--cached",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--ignore-submodules=none",
            "--binary",
            "--unified=2",
            "--",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        cwd,
        Environment::Additions(environment),
    )?;
    let decision = approval.decide(&action, context.deadline())?;
    let policy = Policy::new(1)?;
    let approved = Broker::authorize(action, context, &policy, decision, Instant::now())?;
    let result = execute_process(approved, context)?;
    if result.exit_code != Some(0) {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "Git could not read the staged diff",
        ));
    }
    if result.stdout_truncated {
        return Err(KoruError::new(
            ErrorCode::BudgetExhausted,
            "staged diff exceeds the 64 KiB snapshot limit",
        ));
    }
    let diff = std::str::from_utf8(&result.stdout).map_err(|_| {
        KoruError::new(
            ErrorCode::UnsupportedCapability,
            "staged diff is not valid UTF-8; Koru cannot send it to Lua safely",
        )
    })?;
    let changes = parse_changes(diff)?;
    if changes.is_empty() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "there are no staged changes to plan",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(repository_identity);
    hasher.update(&result.stdout);
    let digest = hasher.finalize().into();
    Ok(Snapshot { changes, digest })
}

#[cfg(target_os = "linux")]
fn git_executable() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into());
    if path.as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err(KoruError::new(
            ErrorCode::BudgetExhausted,
            "PATH exceeds the Git lookup limit",
        ));
    }
    for directory in std::env::split_paths(&path).take(128) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let candidate = directory.join("git");
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
            return Ok(candidate);
        }
    }
    Err(KoruError::new(
        ErrorCode::UnsupportedCapability,
        "Git executable was not found in PATH",
    ))
}

#[cfg(not(target_os = "linux"))]
fn git_executable() -> Result<PathBuf> {
    Err(KoruError::new(
        ErrorCode::UnsupportedCapability,
        "Git snapshots are currently supported only on Linux",
    ))
}

fn parse_changes(diff: &str) -> Result<Vec<Change>> {
    let mut sections = Vec::<String>::new();
    let mut current = String::new();
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if !current.is_empty() {
                sections.push(std::mem::take(&mut current));
            }
        } else if current.is_empty() {
            return Err(KoruError::new(
                ErrorCode::UnsupportedCapability,
                "Git returned an unrecognized staged diff format",
            ));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }
    if sections.len() > MAX_CHANGES {
        return Err(KoruError::new(
            ErrorCode::BudgetExhausted,
            "staged diff contains more than 64 file changes",
        ));
    }
    let mut changes = Vec::with_capacity(sections.len());
    let mut ids = std::collections::BTreeSet::new();
    for section in sections {
        let hash = Sha256::digest(section.as_bytes());
        let id = format!(
            "chg_{}",
            hash[..10]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        if !ids.insert(id.clone()) {
            return Err(KoruError::new(
                ErrorCode::StateConflict,
                "staged change identifier collision",
            ));
        }
        let (kind, excerpt) = describe(&section);
        changes.push(Change { id, kind, excerpt });
    }
    Ok(changes)
}

fn describe(section: &str) -> (String, String) {
    let kind = if section.contains("new file mode ") {
        "added"
    } else if section.contains("deleted file mode ") {
        "deleted"
    } else if section.contains("rename from ") {
        "renamed"
    } else if section.contains("GIT binary patch") || section.contains("Binary files ") {
        "binary"
    } else {
        "modified"
    };
    let mut excerpt = String::new();
    let mut in_hunk = false;
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut binary = false;
    for line in section.lines() {
        if line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
            binary = true;
        }
        if line.starts_with("@@") {
            in_hunk = true;
        }
        if !in_hunk {
            continue;
        }
        if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
        if !binary && excerpt.len() < MAX_EXCERPT_BYTES {
            excerpt.push_str(line);
            excerpt.push('\n');
        }
    }
    let excerpt = if binary {
        "[binary contents omitted]".to_owned()
    } else {
        let mut excerpt = excerpt;
        let mut end = MAX_EXCERPT_BYTES.min(excerpt.len());
        while !excerpt.is_char_boundary(end) {
            end -= 1;
        }
        excerpt.truncate(end);
        redact_provider_credentials(&mut excerpt);
        let mut end = MAX_EXCERPT_BYTES.min(excerpt.len());
        while !excerpt.is_char_boundary(end) {
            end -= 1;
        }
        excerpt.truncate(end);
        excerpt
    };
    (format!("{kind}: +{additions} -{deletions} lines"), excerpt)
}

fn redact_provider_credentials(text: &mut String) {
    for name in ["DEEPSEEK_API_KEY", "OPENCODE_API_KEY"] {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            *text = text.replace(&value, "[REDACTED]");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_CHANGES, parse_changes};

    #[test]
    fn change_ids_hide_paths_and_excerpts_hide_patch_headers() {
        let diff = "diff --git a/private-name.txt b/private-name.txt\nindex 123..456 100644\n--- a/private-name.txt\n+++ b/private-name.txt\n@@ -1 +1 @@\n-old value\n+new value\n";
        let changes = parse_changes(diff).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(!changes[0].id.contains("private-name"));
        assert!(!changes[0].excerpt.contains("private-name"));
        assert!(changes[0].excerpt.contains("new value"));
    }

    #[test]
    fn staged_change_count_is_bounded() {
        let diff = (0..=MAX_CHANGES)
            .map(|index| format!("diff --git a/{index} b/{index}\n@@ -0,0 +1 @@\n+x\n"))
            .collect::<String>();
        assert_eq!(
            parse_changes(&diff).unwrap_err().code(),
            crate::error::ErrorCode::BudgetExhausted
        );
    }

    #[test]
    fn quoted_unusual_paths_stay_out_of_lua_change_data() {
        let diff = "diff --git \"a/file with\\nnewline.txt\" \"b/file with\\nnewline.txt\"\nindex 123..456 100644\n--- \"a/file with\\nnewline.txt\"\n+++ \"b/file with\\nnewline.txt\"\n@@ -1 +1 @@\n-old\n+new\n";
        let changes = parse_changes(diff).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(!changes[0].excerpt.contains("file with"));
    }
}
