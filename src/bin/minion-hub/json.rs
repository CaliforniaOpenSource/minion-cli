//! Tiny JSON helpers for the private API.
//!
//! Requests intentionally accept only flat objects with string values.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;

pub(crate) fn parse_json_object(input: &str) -> Result<BTreeMap<String, String>> {
    let bytes = input.as_bytes();
    let mut idx = 0;
    skip_json_ws(bytes, &mut idx);
    expect_json_byte(bytes, &mut idx, b'{')?;
    skip_json_ws(bytes, &mut idx);

    let mut object = BTreeMap::new();
    if peek_json_byte(bytes, idx) == Some(b'}') {
        idx += 1;
        skip_json_ws(bytes, &mut idx);
        if idx != bytes.len() {
            bail!("unexpected data after JSON object");
        }
        return Ok(object);
    }

    loop {
        let key = parse_json_string(bytes, &mut idx)?;
        skip_json_ws(bytes, &mut idx);
        expect_json_byte(bytes, &mut idx, b':')?;
        skip_json_ws(bytes, &mut idx);
        let value = parse_json_string(bytes, &mut idx)?;
        object.insert(key, value);
        skip_json_ws(bytes, &mut idx);

        match peek_json_byte(bytes, idx) {
            Some(b',') => {
                idx += 1;
                skip_json_ws(bytes, &mut idx);
            }
            Some(b'}') => {
                idx += 1;
                break;
            }
            _ => bail!("expected comma or end of JSON object"),
        }
    }

    skip_json_ws(bytes, &mut idx);
    if idx != bytes.len() {
        bail!("unexpected data after JSON object");
    }
    Ok(object)
}

fn parse_json_string(bytes: &[u8], idx: &mut usize) -> Result<String> {
    expect_json_byte(bytes, idx, b'"')?;
    let mut value = String::new();

    while let Some(byte) = peek_json_byte(bytes, *idx) {
        *idx += 1;
        match byte {
            b'"' => return Ok(value),
            b'\\' => {
                let escaped = peek_json_byte(bytes, *idx).context("unterminated JSON escape")?;
                *idx += 1;
                match escaped {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'/' => value.push('/'),
                    b'b' => value.push('\u{0008}'),
                    b'f' => value.push('\u{000c}'),
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    _ => bail!("unsupported JSON escape"),
                }
            }
            0..=31 => bail!("control character in JSON string"),
            32..=126 => value.push(byte as char),
            _ => bail!("only ASCII JSON strings are supported"),
        }
    }

    bail!("unterminated JSON string")
}

fn skip_json_ws(bytes: &[u8], idx: &mut usize) {
    while matches!(
        peek_json_byte(bytes, *idx),
        Some(b' ' | b'\n' | b'\r' | b'\t')
    ) {
        *idx += 1;
    }
}

fn expect_json_byte(bytes: &[u8], idx: &mut usize, expected: u8) -> Result<()> {
    match peek_json_byte(bytes, *idx) {
        Some(actual) if actual == expected => {
            *idx += 1;
            Ok(())
        }
        _ => bail!("expected JSON byte {}", expected as char),
    }
}

fn peek_json_byte(bytes: &[u8], idx: usize) -> Option<u8> {
    bytes.get(idx).copied()
}

pub(crate) fn json_escape(input: &str) -> String {
    let mut escaped = String::new();
    for ch in input.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_string_objects() {
        let object = parse_json_object(r#"{ "name": "web-01", "vpn_ip": "10.42.42.2" }"#).unwrap();

        assert_eq!(object.get("name").unwrap(), "web-01");
        assert_eq!(object.get("vpn_ip").unwrap(), "10.42.42.2");
    }

    #[test]
    fn rejects_non_string_values() {
        assert!(parse_json_object(r#"{"vpn_ip": 10}"#).is_err());
    }

    #[test]
    fn escapes_response_strings() {
        assert_eq!(json_escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }
}
