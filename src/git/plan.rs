//! Validate an AI grouping against the owner-held staged change IDs.
use crate::{
    error::{ErrorCode, KoruError, Result},
    json::JsonValue,
};
use std::collections::BTreeSet;

/// Validate complete ID coverage and Conventional Commit subjects.
pub(crate) fn validate_plan(known: &BTreeSet<String>, proposal: &JsonValue) -> Result<JsonValue> {
    only_fields(proposal, &["groups"])?;
    let JsonValue::Array(groups) = object_field(proposal, "groups")? else {
        return Err(invalid("groups must be an array"));
    };
    if groups.is_empty() || groups.len() > known.len() {
        return Err(invalid(
            "plan must contain between one and one group per staged change",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut accepted = Vec::with_capacity(groups.len());
    for group in groups {
        only_fields(group, &["changes", "message", "rationale"])?;
        let JsonValue::Array(ids) = object_field(group, "changes")? else {
            return Err(invalid("group changes must be an array"));
        };
        if ids.is_empty() {
            return Err(invalid("a commit group cannot be empty"));
        }
        let mut accepted_ids = Vec::with_capacity(ids.len());
        for id in ids {
            let id = as_string(id, "change id")?;
            if !known.contains(id) {
                return Err(invalid("plan contains an unknown staged change ID"));
            }
            if !seen.insert(id.to_owned()) {
                return Err(invalid("plan repeats a staged change ID"));
            }
            accepted_ids.push(JsonValue::String(id.to_owned()));
        }
        let message = as_string(object_field(group, "message")?, "message")?;
        validate_message(message)?;
        let rationale = match field(group, "rationale") {
            None => String::new(),
            Some(value) => as_string(value, "rationale")?.to_owned(),
        };
        if rationale.len() > 512 || rationale.chars().any(char::is_control) {
            return Err(invalid(
                "group rationale must be at most 512 printable bytes",
            ));
        }
        accepted.push(JsonValue::Object(
            [
                ("changes".to_owned(), JsonValue::Array(accepted_ids)),
                ("message".to_owned(), JsonValue::String(message.to_owned())),
                ("rationale".to_owned(), JsonValue::String(rationale)),
            ]
            .into(),
        ));
    }
    if &seen != known {
        return Err(invalid("plan must cover every staged change exactly once"));
    }
    Ok(JsonValue::Object(
        [
            (
                "status".to_owned(),
                JsonValue::String("plan_only".to_owned()),
            ),
            ("groups".to_owned(), JsonValue::Array(accepted)),
            (
                "change_count".to_owned(),
                JsonValue::Integer(known.len() as i64),
            ),
        ]
        .into(),
    ))
}

fn validate_message(message: &str) -> Result<()> {
    if message.is_empty() || message.len() > 72 || message.chars().any(char::is_control) {
        return Err(invalid("commit message must be 1 to 72 printable bytes"));
    }
    let Some((prefix, subject)) = message.split_once(": ") else {
        return Err(invalid(
            "commit message must use Conventional Commit syntax",
        ));
    };
    if subject.trim().is_empty() {
        return Err(invalid("commit subject must not be empty"));
    }
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let kind = if let Some((kind, scope)) = prefix.split_once('(') {
        let Some(scope) = scope.strip_suffix(')') else {
            return Err(invalid("commit scope must be parenthesized"));
        };
        if scope.is_empty()
            || scope.bytes().any(|byte| byte == b'(' || byte == b')')
            || !scope.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/')
            })
        {
            return Err(invalid("commit scope contains invalid characters"));
        }
        kind
    } else {
        prefix
    };
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        || !kind.as_bytes()[0].is_ascii_lowercase()
    {
        return Err(invalid(
            "commit type must be lowercase Conventional Commit syntax",
        ));
    }
    Ok(())
}

fn object_field<'a>(value: &'a JsonValue, name: &str) -> Result<&'a JsonValue> {
    field(value, name).ok_or_else(|| invalid(format!("plan needs {name:?}")))
}

fn field<'a>(value: &'a JsonValue, name: &str) -> Option<&'a JsonValue> {
    match value {
        JsonValue::Object(fields) => fields.get(name),
        _ => None,
    }
}

fn only_fields(value: &JsonValue, allowed: &[&str]) -> Result<()> {
    let JsonValue::Object(fields) = value else {
        return Ok(());
    };
    if fields
        .keys()
        .any(|field| !allowed.contains(&field.as_str()))
    {
        return Err(invalid("plan contains an unsupported field"));
    }
    Ok(())
}

fn as_string<'a>(value: &'a JsonValue, name: &str) -> Result<&'a str> {
    match value {
        JsonValue::String(value) => Ok(value),
        _ => Err(invalid(format!("{name} must be a string"))),
    }
}

fn invalid(message: impl Into<String>) -> KoruError {
    KoruError::new(ErrorCode::Validation, message)
}

#[cfg(test)]
mod tests {
    use super::validate_plan;
    use crate::{error::ErrorCode, json::JsonValue};
    use std::collections::BTreeSet;

    fn proposal(changes: &[&str], message: &str) -> JsonValue {
        JsonValue::Object(
            [(
                "groups".to_owned(),
                JsonValue::Array(vec![JsonValue::Object(
                    [
                        (
                            "changes".to_owned(),
                            JsonValue::Array(
                                changes
                                    .iter()
                                    .map(|id| JsonValue::String((*id).to_owned()))
                                    .collect(),
                            ),
                        ),
                        ("message".to_owned(), JsonValue::String(message.to_owned())),
                    ]
                    .into(),
                )]),
            )]
            .into(),
        )
    }

    #[test]
    fn accepts_complete_conventional_plan() {
        let known = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
        let value =
            validate_plan(&known, &proposal(&["a", "b"], "feat(cli): group files")).unwrap();
        let JsonValue::Object(fields) = value else {
            panic!("object")
        };
        assert_eq!(
            fields.get("status"),
            Some(&JsonValue::String("plan_only".into()))
        );
    }

    #[test]
    fn rejects_duplicate_unknown_missing_ids_and_invalid_messages() {
        let known = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
        for plan in [
            proposal(&["a", "a"], "feat: duplicate"),
            proposal(&["a", "x"], "feat: unknown"),
            proposal(&["a"], "feat: missing"),
            proposal(&["a", "b"], "Feature: invalid type"),
        ] {
            assert_eq!(
                validate_plan(&known, &plan).unwrap_err().code(),
                ErrorCode::Validation
            );
        }
    }

    #[test]
    fn rejects_paths_and_commands_outside_the_plan_contract() {
        let known = BTreeSet::from(["a".to_owned()]);
        let JsonValue::Object(mut value) = proposal(&["a"], "feat: safe plan") else {
            unreachable!()
        };
        value.insert("path".to_owned(), JsonValue::String("/tmp/file".to_owned()));
        assert_eq!(
            validate_plan(&known, &JsonValue::Object(value))
                .unwrap_err()
                .code(),
            ErrorCode::Validation
        );

        let JsonValue::Object(mut value) = proposal(&["a"], "feat: safe plan") else {
            unreachable!()
        };
        let JsonValue::Array(groups) = value.get_mut("groups").unwrap() else {
            unreachable!()
        };
        let JsonValue::Object(group) = groups.first_mut().unwrap() else {
            unreachable!()
        };
        group.insert(
            "command".to_owned(),
            JsonValue::String("git reset".to_owned()),
        );
        assert_eq!(
            validate_plan(&known, &JsonValue::Object(value))
                .unwrap_err()
                .code(),
            ErrorCode::Validation
        );
    }
}
