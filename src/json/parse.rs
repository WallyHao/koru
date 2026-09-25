//! Strict, bounded JSON parsing into [`JsonValue`].
//!
//! Accepts RFC 8259 JSON with bounded depth, element count, and byte size.
//! Rejects trailing data, duplicate object keys, unescaped control characters,
//! invalid or lone-surrogate escapes, and non-finite or malformed numbers. Input
//! must be valid UTF-8.
use super::{JsonLimits, JsonValue, MAX_SAFE_INTEGER};
use crate::error::{ErrorCode, KoruError, Result};

/// Parse one JSON document under the supplied limits.
pub fn parse(bytes: &[u8], limits: &JsonLimits) -> Result<JsonValue> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("JSON input must be valid UTF-8"))?;
    limits.check_bytes(text.len())?;
    let mut parser = Parser {
        text: text.as_bytes(),
        pos: 0,
        limits: *limits,
        elements: 0,
        bytes: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.pos != parser.text.len() {
        return Err(invalid("trailing data after the JSON value"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
    limits: JsonLimits,
    elements: usize,
    bytes: usize,
}

impl Parser<'_> {
    fn value(&mut self, depth: usize) -> Result<JsonValue> {
        self.limits.check_depth(depth)?;
        let byte = self
            .peek()
            .ok_or_else(|| invalid("unexpected end of JSON input"))?;
        match byte {
            b'n' => {
                self.literal(b"null")?;
                self.charge(0)?;
                Ok(JsonValue::Null)
            }
            b't' => {
                self.literal(b"true")?;
                self.charge(0)?;
                Ok(JsonValue::Bool(true))
            }
            b'f' => {
                self.literal(b"false")?;
                self.charge(0)?;
                Ok(JsonValue::Bool(false))
            }
            b'"' => {
                let text = self.string(false)?;
                self.charge(0)?;
                Ok(JsonValue::String(text))
            }
            b'[' => self.array(depth),
            b'{' => self.object(depth),
            b'-' | b'0'..=b'9' => {
                let number = self.number()?;
                self.charge(0)?;
                Ok(number)
            }
            other => Err(invalid(format!(
                "unexpected byte {:?} in JSON input",
                other as char
            ))),
        }
    }

    fn array(&mut self, depth: usize) -> Result<JsonValue> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(invalid("expected `,` or `]` in JSON array")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<JsonValue> {
        self.expect(b'{')?;
        let mut entries = std::collections::BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string(true)?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            if entries.insert(key, value).is_some() {
                return Err(invalid("JSON object contains a duplicate key"));
            }
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(entries));
                }
                _ => return Err(invalid("expected `,` or `}` in JSON object")),
            }
        }
    }

    fn number(&mut self) -> Result<JsonValue> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.consume_digits(),
            _ => return Err(invalid("invalid JSON number")),
        }
        let mut float = false;
        if self.peek() == Some(b'.') {
            float = true;
            self.pos += 1;
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(invalid("JSON fraction needs digits"));
            }
            self.consume_digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(invalid("JSON exponent needs digits"));
            }
            self.consume_digits();
        }
        let slice = std::str::from_utf8(&self.text[start..self.pos])
            .map_err(|_| invalid("invalid JSON number"))?;
        if !float
            && let Ok(integer) = slice.parse::<i64>()
            && (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&integer)
        {
            return Ok(JsonValue::Integer(integer));
        }
        let number: f64 = slice.parse().map_err(|_| invalid("invalid JSON number"))?;
        if !number.is_finite() {
            return Err(invalid("JSON numbers must be finite"));
        }
        Ok(JsonValue::Number(number))
    }

    fn string(&mut self, key: bool) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(invalid("unterminated JSON string"));
            };
            match byte {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\\' => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                byte if byte < 0x20 => {
                    return Err(invalid("JSON strings must escape control characters"));
                }
                _ => {
                    let rest = std::str::from_utf8(&self.text[self.pos..])
                        .map_err(|_| invalid("JSON strings must be valid UTF-8"))?;
                    let ch = rest
                        .chars()
                        .next()
                        .ok_or_else(|| invalid("unterminated JSON string"))?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        if key {
            self.limits.check_key_bytes(out.len())?;
        } else {
            self.limits.check_string_bytes(out.len())?;
        }
        self.charge(out.len())?;
        Ok(out)
    }

    fn escape(&mut self, out: &mut String) -> Result<()> {
        let byte = self
            .peek()
            .ok_or_else(|| invalid("unterminated JSON escape"))?;
        self.pos += 1;
        match byte {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let code = self.hex4()?;
                let ch = if (0xD800..=0xDBFF).contains(&code) {
                    if self.peek() != Some(b'\\') {
                        return Err(invalid("lone high surrogate in JSON string"));
                    }
                    self.pos += 1;
                    if self.peek() != Some(b'u') {
                        return Err(invalid("lone high surrogate in JSON string"));
                    }
                    self.pos += 1;
                    let low = self.hex4()?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err(invalid("lone high surrogate in JSON string"));
                    }
                    let combined = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                    char::from_u32(combined)
                        .ok_or_else(|| invalid("invalid JSON surrogate pair"))?
                } else if (0xDC00..=0xDFFF).contains(&code) {
                    return Err(invalid("lone low surrogate in JSON string"));
                } else {
                    char::from_u32(code).ok_or_else(|| invalid("invalid JSON escape"))?
                };
                out.push(ch);
            }
            _ => return Err(invalid("invalid escape in JSON string")),
        }
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32> {
        if self.pos + 4 > self.text.len() {
            return Err(invalid("truncated JSON unicode escape"));
        }
        let mut value = 0_u32;
        for _ in 0..4 {
            let byte = self.text[self.pos];
            self.pos += 1;
            let digit = (byte as char)
                .to_digit(16)
                .ok_or_else(|| invalid("invalid JSON unicode escape"))?;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn literal(&mut self, expected: &[u8]) -> Result<()> {
        if self.text[self.pos..].starts_with(expected) {
            self.pos += expected.len();
            Ok(())
        } else {
            Err(invalid("invalid JSON literal"))
        }
    }

    fn consume_digits(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(invalid(format!(
                "expected `{}` in JSON input",
                byte as char
            )))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.elements = self.elements.saturating_add(1);
        self.limits.check_elements(self.elements)?;
        self.bytes = self.bytes.saturating_add(bytes);
        self.limits.check_bytes(self.bytes)
    }
}

fn invalid(detail: impl Into<String>) -> KoruError {
    KoruError::new(ErrorCode::Validation, detail)
}
