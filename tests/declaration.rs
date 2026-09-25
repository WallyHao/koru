//! Pure declaration contract tests: `CommandDeclaration::new` is the validator.
use koru::{
    declaration::{
        Argument, ArgumentType, ArgumentValue, Capabilities, CommandDeclaration,
        MAX_DESCRIPTION_BYTES, MAX_SAFE_INTEGER, MAX_TOOLS, ToolDeclaration,
    },
    error::{ErrorCode, Result},
    json::JsonValue,
};

fn argument(name: &str, kind: ArgumentType) -> Argument {
    Argument {
        name: name.to_owned(),
        kind,
        required: false,
        default: None,
        help: None,
        min: None,
        max: None,
        max_len: None,
    }
}
fn build(arguments: Vec<Argument>) -> Result<CommandDeclaration> {
    CommandDeclaration::new(1, "demo", arguments, Capabilities::default(), Vec::new())
}
fn code(result: Result<CommandDeclaration>) -> ErrorCode {
    result.unwrap_err().code()
}

#[test]
fn accepts_a_minimal_declaration() {
    let declaration = build(Vec::new()).unwrap();
    assert_eq!(declaration.api_version(), 1);
    assert_eq!(declaration.description(), "demo");
    assert!(declaration.arguments().is_empty());
    assert!(!declaration.capabilities().direct_processes);
}

#[test]
fn rejects_unsupported_api_versions() {
    for version in [0, 2, u32::MAX] {
        assert_eq!(
            CommandDeclaration::new(
                version,
                "demo",
                Vec::new(),
                Capabilities::default(),
                Vec::new(),
            )
            .unwrap_err()
            .code(),
            ErrorCode::UnsupportedCapability
        );
    }
}

#[test]
fn rejects_missing_or_malformed_descriptions() {
    for description in [
        String::new(),
        "bad\u{7}".to_owned(),
        "x".repeat(MAX_DESCRIPTION_BYTES + 1),
    ] {
        assert_eq!(
            CommandDeclaration::new(
                1,
                description,
                Vec::new(),
                Capabilities::default(),
                Vec::new(),
            )
            .unwrap_err()
            .code(),
            ErrorCode::Validation
        );
    }
}

#[test]
fn rejects_duplicate_or_malformed_argument_names() {
    assert_eq!(
        code(build(vec![
            argument("task", ArgumentType::String),
            argument("task", ArgumentType::String),
        ])),
        ErrorCode::Validation
    );
    for name in ["Task", "", "1task", &"x".repeat(65)] {
        assert_eq!(
            code(build(vec![argument(name, ArgumentType::Boolean)])),
            ErrorCode::Validation
        );
    }
}

#[test]
fn rejects_required_arguments_with_defaults() {
    let mut item = argument("count", ArgumentType::Integer);
    item.required = true;
    item.default = Some(ArgumentValue::Integer(1));
    assert_eq!(code(build(vec![item])), ErrorCode::Validation);
}

#[test]
fn rejects_defaults_that_do_not_match_the_type() {
    let mut item = argument("count", ArgumentType::Integer);
    item.default = Some(ArgumentValue::String("nope".to_owned()));
    assert_eq!(code(build(vec![item])), ErrorCode::Validation);

    let mut enum_item = argument("mode", ArgumentType::Enum(vec!["fast".to_owned()]));
    enum_item.default = Some(ArgumentValue::String("slow".to_owned()));
    assert_eq!(code(build(vec![enum_item])), ErrorCode::Validation);
}

#[test]
fn rejects_integers_beyond_the_portable_range() {
    let mut item = argument("count", ArgumentType::Integer);
    item.default = Some(ArgumentValue::Integer(MAX_SAFE_INTEGER + 1));
    assert_eq!(code(build(vec![item])), ErrorCode::Validation);

    let mut valid = argument("count", ArgumentType::Integer);
    valid.default = Some(ArgumentValue::Integer(-MAX_SAFE_INTEGER));
    assert!(build(vec![valid]).is_ok());
}

#[test]
fn rejects_reversed_nonfinite_and_mistyped_bounds() {
    let mut reversed = argument("count", ArgumentType::Integer);
    reversed.min = Some(10.0);
    reversed.max = Some(1.0);
    assert_eq!(code(build(vec![reversed])), ErrorCode::Validation);

    let mut nonfinite = argument("ratio", ArgumentType::Number);
    nonfinite.min = Some(f64::INFINITY);
    assert_eq!(code(build(vec![nonfinite])), ErrorCode::Validation);

    let mut mistyped = argument("flag", ArgumentType::Boolean);
    mistyped.min = Some(0.0);
    assert_eq!(code(build(vec![mistyped])), ErrorCode::Validation);

    let mut max_len = argument("count", ArgumentType::Integer);
    max_len.max_len = Some(4);
    assert_eq!(code(build(vec![max_len])), ErrorCode::Validation);
}

#[test]
fn rejects_empty_and_duplicate_enum_values() {
    assert_eq!(
        code(build(vec![argument(
            "mode",
            ArgumentType::Enum(Vec::new())
        )])),
        ErrorCode::Validation
    );
    assert_eq!(
        code(build(vec![argument(
            "mode",
            ArgumentType::Enum(vec!["a".to_owned(), "a".to_owned()])
        )])),
        ErrorCode::Validation
    );
}

#[test]
fn rejects_too_many_arguments() {
    let arguments = (0..=33)
        .map(|index| argument(&format!("a{index}"), ArgumentType::Boolean))
        .collect();
    assert_eq!(code(build(arguments)), ErrorCode::Validation);
}

#[test]
fn accepts_a_fully_specified_argument() {
    let mut count = argument("count", ArgumentType::Integer);
    count.default = Some(ArgumentValue::Integer(3));
    count.help = Some("How many times".to_owned());
    count.min = Some(1.0);
    count.max = Some(10.0);
    let mut mode = argument(
        "mode",
        ArgumentType::Enum(vec!["fast".to_owned(), "slow".to_owned()]),
    );
    mode.default = Some(ArgumentValue::String("fast".to_owned()));
    mode.required = false;
    assert!(build(vec![count, mode]).is_ok());
}

#[test]
fn carries_requested_capabilities_without_granting_them() {
    let declaration = CommandDeclaration::new(
        1,
        "demo",
        Vec::new(),
        Capabilities {
            direct_processes: true,
        },
        Vec::new(),
    )
    .unwrap();
    assert!(declaration.capabilities().direct_processes);
}

fn tool(name: &str) -> ToolDeclaration {
    ToolDeclaration {
        name: name.to_owned(),
        description: "does a thing".to_owned(),
        parameters: JsonValue::Object(Default::default()),
        result: None,
    }
}
fn build_with_tools(tools: Vec<ToolDeclaration>) -> Result<CommandDeclaration> {
    CommandDeclaration::new(1, "demo", Vec::new(), Capabilities::default(), tools)
}

#[test]
fn accepts_valid_tools() {
    let declaration = build_with_tools(vec![tool("first"), tool("second")]).unwrap();
    assert_eq!(declaration.tools().len(), 2);
    assert_eq!(declaration.tools()[0].name, "first");
}

#[test]
fn rejects_duplicate_or_malformed_tool_names() {
    assert_eq!(
        build_with_tools(vec![tool("same"), tool("same")])
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    for name in ["Bad", "", "1tool"] {
        assert_eq!(
            build_with_tools(vec![tool(name)]).unwrap_err().code(),
            ErrorCode::Validation
        );
    }
}

#[test]
fn rejects_non_object_tool_schemas() {
    let mut parameters = tool("t");
    parameters.parameters = JsonValue::String("nope".to_owned());
    assert_eq!(
        build_with_tools(vec![parameters]).unwrap_err().code(),
        ErrorCode::Validation
    );
    let mut result = tool("t");
    result.result = Some(JsonValue::Bool(true));
    assert_eq!(
        build_with_tools(vec![result]).unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn rejects_too_many_tools() {
    let tools = (0..=MAX_TOOLS)
        .map(|index| tool(&format!("t{index}")))
        .collect();
    assert_eq!(
        build_with_tools(tools).unwrap_err().code(),
        ErrorCode::Validation
    );
}
