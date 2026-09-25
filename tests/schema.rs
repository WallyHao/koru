//! JSON Schema subset: compile/validate matrix and deterministic fuzz.
use koru::{
    error::{ErrorCode, Result},
    json::{JsonLimits, JsonValue},
    schema::JsonSchema,
};
use std::collections::BTreeMap;

fn object(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
fn arr(items: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(items)
}
fn string(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}
fn compile(document: &JsonValue) -> Result<JsonSchema> {
    JsonSchema::compile(document, &JsonLimits::default())
}

#[test]
fn accepts_the_documented_subset() {
    let document = object(vec![
        ("type", string("object")),
        (
            "properties",
            object(vec![
                (
                    "name",
                    object(vec![
                        ("type", string("string")),
                        ("minLength", JsonValue::Integer(1)),
                        ("maxLength", JsonValue::Integer(64)),
                    ]),
                ),
                (
                    "count",
                    object(vec![
                        ("type", string("integer")),
                        ("minimum", JsonValue::Integer(0)),
                        ("maximum", JsonValue::Integer(10)),
                    ]),
                ),
                (
                    "mode",
                    object(vec![("enum", arr(vec![string("a"), string("b")]))]),
                ),
                ("flag", object(vec![("type", string("boolean"))])),
                ("nothing", object(vec![("type", string("null"))])),
                (
                    "tags",
                    object(vec![
                        ("type", string("array")),
                        ("items", object(vec![("type", string("string"))])),
                        ("minItems", JsonValue::Integer(0)),
                        ("maxItems", JsonValue::Integer(4)),
                    ]),
                ),
            ]),
        ),
        ("required", arr(vec![string("name")])),
        ("additionalProperties", JsonValue::Bool(false)),
        ("description", string("an example schema")),
    ]);
    assert!(compile(&document).is_ok());
}

#[test]
fn accepts_a_finite_number_bound() {
    let document = object(vec![
        ("type", string("number")),
        ("minimum", JsonValue::Number(0.5)),
        ("maximum", JsonValue::Number(2.5)),
    ]);
    assert!(compile(&document).is_ok());
}

#[test]
fn rejects_unknown_and_remote_keywords() {
    for keyword in [
        "$ref",
        "$schema",
        "$id",
        "oneOf",
        "anyOf",
        "allOf",
        "not",
        "patternProperties",
        "propertyNames",
        "pattern",
        "format",
        "default",
        "examples",
        "const",
    ] {
        let document = object(vec![
            ("type", string("object")),
            (keyword, JsonValue::Bool(true)),
        ]);
        let error = compile(&document).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Validation, "keyword {keyword}");
        assert!(error.message().contains(keyword), "message names {keyword}");
    }
}

#[test]
fn validates_objects_required_and_additional_properties() {
    let schema = compile(&object(vec![
        ("type", string("object")),
        (
            "properties",
            object(vec![("name", object(vec![("type", string("string"))]))]),
        ),
        ("required", arr(vec![string("name")])),
        ("additionalProperties", JsonValue::Bool(false)),
    ]))
    .unwrap();
    assert!(
        schema
            .validate(&object(vec![("name", string("ok"))]))
            .is_ok()
    );
    assert_eq!(
        schema
            .validate(&object(vec![
                ("name", string("ok")),
                ("extra", JsonValue::Null)
            ]))
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    let missing = schema.validate(&object(vec![])).unwrap_err();
    assert!(missing.message().contains("name"));
    let wrong = schema
        .validate(&object(vec![("name", JsonValue::Integer(1))]))
        .unwrap_err();
    assert!(wrong.message().contains("name"));
}

#[test]
fn validates_nested_array_items_and_bounds() {
    let schema = compile(&object(vec![
        ("type", string("object")),
        (
            "properties",
            object(vec![(
                "values",
                object(vec![
                    ("type", string("array")),
                    ("items", object(vec![("type", string("integer"))])),
                    ("minItems", JsonValue::Integer(1)),
                    ("maxItems", JsonValue::Integer(2)),
                ]),
            )]),
        ),
    ]))
    .unwrap();
    assert!(
        schema
            .validate(&object(vec![(
                "values",
                arr(vec![JsonValue::Integer(1), JsonValue::Integer(2)]),
            )]))
            .is_ok()
    );
    assert_eq!(
        schema
            .validate(&object(vec![("values", arr(vec![]))]))
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    let bad_item = schema
        .validate(&object(vec![(
            "values",
            arr(vec![JsonValue::Integer(1), string("two")]),
        )]))
        .unwrap_err();
    assert!(bad_item.message().contains("/values/1"));
}

#[test]
fn validates_string_length_by_bytes() {
    let schema = compile(&object(vec![
        ("type", string("string")),
        ("minLength", JsonValue::Integer(2)),
        ("maxLength", JsonValue::Integer(4)),
    ]))
    .unwrap();
    assert!(schema.validate(&string("abc")).is_ok());
    assert_eq!(
        schema.validate(&string("a")).unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        schema.validate(&string("abcde")).unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn integers_satisfy_number_but_not_the_reverse() {
    let number = compile(&object(vec![("type", string("number"))])).unwrap();
    assert!(number.validate(&JsonValue::Integer(1)).is_ok());
    assert!(number.validate(&JsonValue::Number(1.5)).is_ok());

    let integer = compile(&object(vec![("type", string("integer"))])).unwrap();
    assert!(integer.validate(&JsonValue::Integer(1)).is_ok());
    assert_eq!(
        integer
            .validate(&JsonValue::Number(1.5))
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
}

#[test]
fn rejects_integer_bounds_that_are_not_safe_whole_numbers() {
    let fractional = object(vec![
        ("type", string("integer")),
        ("minimum", JsonValue::Number(0.5)),
    ]);
    assert_eq!(
        compile(&fractional).unwrap_err().code(),
        ErrorCode::Validation
    );

    let unsafe_bound = object(vec![
        ("type", string("integer")),
        ("maximum", JsonValue::Integer(i64::MAX)),
    ]);
    assert_eq!(
        compile(&unsafe_bound).unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn enum_values_must_match_the_declared_type() {
    let mixed = object(vec![
        ("type", string("string")),
        ("enum", arr(vec![string("a"), JsonValue::Integer(1)])),
    ]);
    assert_eq!(compile(&mixed).unwrap_err().code(), ErrorCode::Validation);

    let duplicates = object(vec![("enum", arr(vec![string("a"), string("a")]))]);
    assert_eq!(
        compile(&duplicates).unwrap_err().code(),
        ErrorCode::Validation
    );

    let empty = object(vec![("type", string("string")), ("enum", arr(vec![]))]);
    assert_eq!(compile(&empty).unwrap_err().code(), ErrorCode::Validation);
}

#[test]
fn supports_type_unions() {
    let schema = compile(&object(vec![(
        "type",
        arr(vec![string("string"), string("null")]),
    )]))
    .unwrap();
    assert!(schema.validate(&string("x")).is_ok());
    assert!(schema.validate(&JsonValue::Null).is_ok());
    assert_eq!(
        schema.validate(&JsonValue::Bool(true)).unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn rejects_keywords_that_conflict_with_the_declared_type() {
    let document = object(vec![
        ("type", string("string")),
        ("minimum", JsonValue::Integer(0)),
    ]);
    assert_eq!(
        compile(&document).unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn object_keywords_imply_an_object_root() {
    let implied = compile(&object(vec![(
        "properties",
        object(vec![("a", object(vec![("type", string("integer"))]))]),
    )]))
    .unwrap();
    assert!(implied.is_object_root());

    let any = compile(&object(vec![])).unwrap();
    assert!(!any.is_object_root());

    let union = compile(&object(vec![(
        "type",
        arr(vec![string("object"), string("null")]),
    )]))
    .unwrap();
    assert!(!union.is_object_root());
}

#[test]
fn path_qualified_errors_do_not_panic_or_leak_size() {
    let schema = compile(&object(vec![
        ("type", string("object")),
        (
            "properties",
            object(vec![(
                "deep",
                object(vec![
                    ("type", string("object")),
                    (
                        "properties",
                        object(vec![("value", object(vec![("type", string("integer"))]))]),
                    ),
                ]),
            )]),
        ),
    ]))
    .unwrap();
    let error = schema
        .validate(&object(vec![(
            "deep",
            object(vec![("value", string("no"))]),
        )]))
        .unwrap_err();
    assert!(error.message().contains("/deep/value"));
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
}

fn random_value(rng: &mut Rng, depth: usize) -> JsonValue {
    let pick = rng.next() % if depth == 0 { 6 } else { 8 };
    match pick {
        0 => JsonValue::Null,
        1 => JsonValue::Bool(rng.next().is_multiple_of(2)),
        2 => JsonValue::Integer((rng.next() % 2000) as i64 - 1000),
        3 => JsonValue::Number((rng.next() % 1000) as f64 / 8.0),
        4 => JsonValue::String(format!("s{}", rng.next() % 50)),
        5 => JsonValue::String(String::new()),
        6 => {
            let length = (rng.next() % 4) as usize;
            let mut items = Vec::with_capacity(length);
            for _ in 0..length {
                items.push(random_value(rng, depth - 1));
            }
            JsonValue::Array(items)
        }
        _ => {
            let length = (rng.next() % 4) as usize;
            let mut entries = BTreeMap::new();
            for index in 0..length {
                entries.insert(format!("k{index}"), random_value(rng, depth - 1));
            }
            JsonValue::Object(entries)
        }
    }
}

fn random_keyword(rng: &mut Rng) -> &'static str {
    const KEYWORDS: [&str; 16] = [
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "minItems",
        "maxItems",
        "minLength",
        "maxLength",
        "minimum",
        "maximum",
        "enum",
        "description",
        "$ref",
        "pattern",
        "oneOf",
    ];
    KEYWORDS[(rng.next() % KEYWORDS.len() as u64) as usize]
}

#[test]
fn fuzzing_never_panics_and_stays_deterministic() {
    let mut rng = Rng(0x5eed);
    for _ in 0..2_000 {
        let mut document = BTreeMap::new();
        let count = (rng.next() % 4) as usize;
        for _ in 0..count {
            document.insert(
                random_keyword(&mut rng).to_owned(),
                random_value(&mut rng, 2),
            );
        }
        let document = JsonValue::Object(document);
        let Ok(schema) = compile(&document) else {
            continue;
        };
        for _ in 0..8 {
            let value = random_value(&mut rng, 3);
            let first = schema.validate(&value);
            let second = schema.validate(&value);
            match (first, second) {
                (Ok(()), Ok(())) => {}
                (Err(a), Err(b)) => {
                    assert_eq!(a.code(), b.code());
                    assert_eq!(a.message(), b.message());
                }
                _ => panic!("validation is not deterministic"),
            }
        }
    }
}
