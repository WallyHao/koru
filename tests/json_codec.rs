//! JSON codec: acceptance, rejection, limits, and canonical round trips.
use koru::{
    error::ErrorCode,
    json::{JsonLimits, JsonValue, emit, parse},
};
use std::collections::BTreeMap;

fn parse_str(text: &str) -> JsonValue {
    parse(text.as_bytes(), &JsonLimits::default()).unwrap()
}
fn parse_code(text: &str, limits: &JsonLimits) -> ErrorCode {
    parse(text.as_bytes(), limits).unwrap_err().code()
}

#[test]
fn parses_scalars_and_containers() {
    assert_eq!(parse_str("null"), JsonValue::Null);
    assert_eq!(parse_str(" true "), JsonValue::Bool(true));
    assert_eq!(parse_str("false"), JsonValue::Bool(false));
    assert_eq!(parse_str("\"hi\""), JsonValue::String("hi".to_owned()));
    assert_eq!(
        parse_str("{\"b\":1,\"a\":[null,true,\"x\"]}"),
        JsonValue::Object(BTreeMap::from([
            (
                "a".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::Null,
                    JsonValue::Bool(true),
                    JsonValue::String("x".to_owned()),
                ])
            ),
            ("b".to_owned(), JsonValue::Integer(1)),
        ]))
    );
}

#[test]
fn handles_escapes_and_surrogate_pairs() {
    assert_eq!(
        parse_str(r#""a\nb\t\"c\\""#),
        JsonValue::String("a\nb\t\"c\\".to_owned())
    );
    assert_eq!(
        parse_str(r#""\u0041\u00e9""#),
        JsonValue::String("Aé".to_owned())
    );
    assert_eq!(
        parse_str(r#""\ud83d\ude00""#),
        JsonValue::String("😀".to_owned())
    );
    assert_eq!(parse_str(r#""\/""#), JsonValue::String("/".to_owned()));
}

#[test]
fn types_numbers_by_exactness() {
    assert_eq!(parse_str("2"), JsonValue::Integer(2));
    assert_eq!(parse_str("-2"), JsonValue::Integer(-2));
    assert_eq!(parse_str("2.0"), JsonValue::Number(2.0));
    assert_eq!(parse_str("2e2"), JsonValue::Number(200.0));
    assert_eq!(parse_str("1.5"), JsonValue::Number(1.5));
    assert_eq!(
        parse_str("9007199254740991"),
        JsonValue::Integer(9007199254740991)
    );
    assert_eq!(
        parse_str("9007199254740993"),
        JsonValue::Number(9007199254740992.0)
    );
}

#[test]
fn rejects_malformed_documents() {
    for text in [
        "",
        "nul",
        "tru",
        "01",
        "1.",
        "1e",
        "1e+",
        "+1",
        "NaN",
        "Infinity",
        "--1",
        "\"unterminated",
        r#""bad\q"#,
        r#""lone\ud800""#,
        r#""lone\udc00""#,
        "{\"a\":1,}",
        "{\"a\"}",
        "{\"a\":1",
        "{\"a\":1,\"a\":2}",
        "[1,]",
        "[1 2]",
        "[1",
        "{} {}",
        "1 2",
        "tru e",
    ] {
        assert_eq!(
            parse_code(text, &JsonLimits::default()),
            ErrorCode::Validation,
            "text: {text:?}"
        );
    }
}

#[test]
fn rejects_invalid_utf8_and_raw_control_characters() {
    assert_eq!(
        parse(&[0xff, 0xfe], &JsonLimits::default())
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    assert_eq!(
        parse(b"\"raw\x1fcontrol\"", &JsonLimits::default())
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
}

#[test]
fn enforces_depth_element_and_byte_limits() {
    let text = "[[1]]";
    assert_eq!(
        parse_code(
            text,
            &JsonLimits {
                max_depth: 1,
                ..JsonLimits::default()
            }
        ),
        ErrorCode::BudgetExhausted
    );
    assert_eq!(
        parse_code(
            "[1,2,3]",
            &JsonLimits {
                max_elements: 1,
                ..JsonLimits::default()
            }
        ),
        ErrorCode::BudgetExhausted
    );
    assert_eq!(
        parse_code(
            "\"toolong\"",
            &JsonLimits {
                max_string_bytes: 1,
                ..JsonLimits::default()
            }
        ),
        ErrorCode::BudgetExhausted
    );
    assert_eq!(
        parse_code(
            "{\"key\":1}",
            &JsonLimits {
                max_key_bytes: 1,
                ..JsonLimits::default()
            }
        ),
        ErrorCode::BudgetExhausted
    );
    assert_eq!(
        parse_code(
            "\"abcd\"",
            &JsonLimits {
                max_bytes: 2,
                ..JsonLimits::default()
            }
        ),
        ErrorCode::BudgetExhausted
    );
}

#[test]
fn emits_canonical_compact_json() {
    let value = JsonValue::Object(BTreeMap::from([
        (
            "b".to_owned(),
            JsonValue::Array(vec![JsonValue::Integer(1), JsonValue::Number(2.0)]),
        ),
        ("a".to_owned(), JsonValue::String("x\"y\n".to_owned())),
    ]));
    assert_eq!(emit(&value), r#"{"a":"x\"y\n","b":[1,2.0]}"#);
    assert_eq!(emit(&JsonValue::Array(Vec::new())), "[]");
    assert_eq!(emit(&JsonValue::Object(BTreeMap::new())), "{}");
    assert_eq!(emit(&JsonValue::Number(2.0)), "2.0");
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
        3 => JsonValue::Number((rng.next() % 100) as f64 / 4.0),
        4 => JsonValue::String(format!("s{}\"'\\\n", rng.next() % 50)),
        5 => JsonValue::String(String::new()),
        6 => {
            let length = (rng.next() % 4) as usize;
            JsonValue::Array((0..length).map(|_| random_value(rng, depth - 1)).collect())
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

#[test]
fn property_round_trips_values() {
    let mut rng = Rng(0x0bad_c0de);
    for _ in 0..1000 {
        let value = random_value(&mut rng, 3);
        let text = emit(&value);
        let back = parse(text.as_bytes(), &JsonLimits::default()).unwrap();
        assert_eq!(back, value, "text: {text}");
    }
    for number in [2.0, -0.0, 1e300, 1e-300, f64::MIN_POSITIVE] {
        let value = JsonValue::Number(number);
        let back = parse(emit(&value).as_bytes(), &JsonLimits::default()).unwrap();
        assert_eq!(back, value);
    }
}
