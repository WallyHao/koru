//! Unit tests for Lua/JSON conversion and its limits.
use super::*;
use crate::{
    error::{ErrorCode, Result},
    json::{JsonLimits, JsonValue},
};
use mlua::{Lua, Value};

fn lua_with_json() -> Lua {
    let lua = Lua::new();
    let json = install(&lua).unwrap();
    lua.globals().set("json", json).unwrap();
    lua
}
fn eval(lua: &Lua, code: &str, limits: &JsonLimits) -> Result<JsonValue> {
    let value: Value = lua.load(code).eval().unwrap();
    to_json(&value, limits)
}
fn default_eval(lua: &Lua, code: &str) -> Result<JsonValue> {
    eval(lua, code, &JsonLimits::default())
}

#[test]
fn round_trips_scalars_and_null() {
    let lua = lua_with_json();
    assert_eq!(default_eval(&lua, "json.null").unwrap(), JsonValue::Null);
    assert_eq!(default_eval(&lua, "true").unwrap(), JsonValue::Bool(true));
    assert_eq!(default_eval(&lua, "42").unwrap(), JsonValue::Integer(42));
    assert_eq!(default_eval(&lua, "1.5").unwrap(), JsonValue::Number(1.5));
    assert_eq!(
        default_eval(&lua, "'hi'").unwrap(),
        JsonValue::String("hi".to_owned())
    );
}

#[test]
fn distinguishes_empty_arrays_and_objects() {
    let lua = lua_with_json();
    assert_eq!(
        default_eval(&lua, "{}").unwrap(),
        JsonValue::Object(Default::default())
    );
    assert_eq!(
        default_eval(&lua, "json.array({})").unwrap(),
        JsonValue::Array(Vec::new())
    );
    assert_eq!(
        default_eval(&lua, "json.object({})").unwrap(),
        JsonValue::Object(Default::default())
    );
}

#[test]
fn round_trips_nested_containers_through_lua() {
    let lua = lua_with_json();
    let source = "{ a = 1, b = { true, json.null }, c = json.array({}) }";
    let json = default_eval(&lua, source).unwrap();
    let back = from_json(&lua, &json).unwrap();
    assert_eq!(to_json(&back, &JsonLimits::default()).unwrap(), json);
}

#[test]
fn rejects_ambiguous_and_mixed_tables() {
    let lua = lua_with_json();
    for code in ["{ 1, 2, x = 3 }", "{ [1] = 1, [3] = 3 }", "{ [0] = 1 }"] {
        assert_eq!(
            default_eval(&lua, code).unwrap_err().code(),
            ErrorCode::Validation
        );
    }
}

#[test]
fn rejects_cycles_and_unsupported_values() {
    let lua = lua_with_json();
    assert_eq!(
        default_eval(&lua, "local t = {} t.self = t return t")
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    assert_eq!(
        default_eval(&lua, "print").unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        default_eval(&lua, "string.char(255)").unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        default_eval(&lua, "nil").unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn rejects_nonfinite_and_unsafe_numbers() {
    let lua = lua_with_json();
    assert_eq!(
        default_eval(&lua, "math.huge").unwrap_err().code(),
        ErrorCode::Validation
    );
    assert_eq!(
        default_eval(&lua, "9007199254740993").unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn enforces_depth_element_and_byte_limits() {
    let lua = lua_with_json();
    let shallow = JsonLimits {
        max_depth: 1,
        ..JsonLimits::default()
    };
    assert_eq!(
        eval(&lua, "{{ 1 }}", &shallow).unwrap_err().code(),
        ErrorCode::BudgetExhausted
    );
    let tiny = JsonLimits {
        max_string_bytes: 1,
        ..JsonLimits::default()
    };
    assert_eq!(
        eval(&lua, "'too long'", &tiny).unwrap_err().code(),
        ErrorCode::BudgetExhausted
    );
    let few = JsonLimits {
        max_elements: 0,
        ..JsonLimits::default()
    };
    assert_eq!(
        eval(&lua, "{1,2,3}", &few).unwrap_err().code(),
        ErrorCode::BudgetExhausted
    );
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

fn random_json(rng: &mut Rng, depth: usize) -> JsonValue {
    let pick = rng.next() % if depth == 0 { 6 } else { 8 };
    match pick {
        0 => JsonValue::Null,
        1 => JsonValue::Bool(rng.next().is_multiple_of(2)),
        2 => JsonValue::Integer((rng.next() % 2000) as i64 - 1000),
        3 => JsonValue::Number((rng.next() % 100) as f64 / 4.0),
        4 => JsonValue::String(format!("s{}", rng.next() % 50)),
        5 => JsonValue::String(String::new()),
        6 => {
            let length = (rng.next() % 4) as usize;
            let items = (0..length)
                .map(|_| random_json(rng, depth - 1))
                .collect::<Vec<_>>();
            JsonValue::Array(items)
        }
        _ => {
            let length = (rng.next() % 4) as usize;
            let mut entries = std::collections::BTreeMap::new();
            for index in 0..length {
                entries.insert(format!("k{index}"), random_json(rng, depth - 1));
            }
            JsonValue::Object(entries)
        }
    }
}

#[test]
fn property_round_trips_generated_values() {
    let lua = lua_with_json();
    let mut rng = Rng(0x1234_5678);
    for _ in 0..500 {
        let value = random_json(&mut rng, 3);
        let lua_value = from_json(&lua, &value).unwrap();
        let back = to_json(&lua_value, &JsonLimits::default()).unwrap();
        assert_eq!(back, value);
    }
}
