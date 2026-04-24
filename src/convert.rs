//! Shared conversion helpers used by multiple store backends.
//!
//! Contains hex encoding/decoding, the `$persist:data` sentinel constant,
//! and `Value` ↔ `serde_json::Value` conversion functions. These are
//! consolidated here so that `WebStore`, `SharedPreferencesStore`, and
//! `serde_compat` all share a single implementation.

use crate::value::Value;

/// Sentinel object key used to represent `Value::Data` inside JSON.
///
/// A single-key JSON object `{"$persist:data": "<hex>"}` encodes binary
/// data for formats (JSON, localStorage, SharedPreferences) that lack a
/// native binary type.
pub(crate) const DATA_SENTINEL: &str = "$persist:data";

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Value ↔ serde_json::Value
//
// Available whenever serde_json is a dependency — which includes the `serde`
// and `json` features, plus the wasm32 and android targets.
// ---------------------------------------------------------------------------

#[cfg(any(feature = "serde", target_arch = "wasm32", target_os = "android"))]
#[allow(dead_code)] // Functions used by platform-gated modules (web.rs, shared_preferences.rs)
mod json {
    use super::*;
    use crate::error::Error;
    use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
    use std::collections::HashMap;

    pub(crate) fn value_to_json(value: &Value) -> JsonValue {
        match value {
            Value::Null => JsonValue::Null,
            Value::Bool(b) => JsonValue::Bool(*b),
            Value::Int(n) => JsonValue::Number((*n).into()),
            Value::Float(n) => JsonNumber::from_f64(*n)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            Value::String(s) => JsonValue::String(s.clone()),
            Value::Data(bytes) => {
                let mut map = JsonMap::with_capacity(1);
                map.insert(
                    DATA_SENTINEL.to_owned(),
                    JsonValue::String(hex_encode(bytes)),
                );
                JsonValue::Object(map)
            }
            Value::Array(arr) => JsonValue::Array(arr.iter().map(value_to_json).collect()),
            Value::Object(obj) => {
                let mut map = JsonMap::with_capacity(obj.len());
                for (k, v) in obj {
                    map.insert(k.clone(), value_to_json(v));
                }
                JsonValue::Object(map)
            }
        }
    }

    pub(crate) fn json_to_value(json: JsonValue) -> Result<Value, Error> {
        Ok(match json {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(b) => Value::Bool(b),
            JsonValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(f) = n.as_f64() {
                    Value::Float(f)
                } else {
                    return Err(Error::Parse(format!("unrepresentable JSON number: {n}")));
                }
            }
            JsonValue::String(s) => Value::String(s),
            JsonValue::Array(arr) => {
                let mut out = Vec::with_capacity(arr.len());
                for item in arr {
                    out.push(json_to_value(item)?);
                }
                Value::Array(out)
            }
            JsonValue::Object(map) => {
                // Detect Data sentinel: single-key object where the key is
                // DATA_SENTINEL and the value is a hex string.
                if map.len() == 1 {
                    if let Some(JsonValue::String(hex)) = map.get(DATA_SENTINEL) {
                        let bytes = hex_decode(hex)
                            .ok_or_else(|| Error::Parse("invalid hex in Data sentinel".into()))?;
                        return Ok(Value::Data(bytes));
                    }
                }
                let mut out = HashMap::with_capacity(map.len());
                for (k, v) in map {
                    out.insert(k, json_to_value(v)?);
                }
                Value::Object(out)
            }
        })
    }

    pub(crate) fn value_to_json_string(value: &Value) -> Result<String, Error> {
        let json = value_to_json(value);
        serde_json::to_string(&json)
            .map_err(|e| Error::Parse(format!("serialize Value to JSON: {e}")))
    }

    pub(crate) fn json_string_to_value(s: &str) -> Result<Value, Error> {
        let json: JsonValue =
            serde_json::from_str(s).map_err(|e| Error::Parse(format!("parse JSON: {e}")))?;
        json_to_value(json)
    }
}

#[cfg(any(feature = "serde", target_arch = "wasm32", target_os = "android"))]
#[allow(unused_imports)]
pub(crate) use json::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encode_all_bytes() {
        let data: Vec<u8> = (0u8..=255u8).collect();
        let s = hex_encode(&data);
        assert_eq!(s.len(), 512);
        assert!(s.starts_with("000102"));
        assert!(s.ends_with("fdfeff"));
    }

    #[test]
    fn hex_round_trip() {
        let data: Vec<u8> = (0u8..=255u8).collect();
        let back = hex_decode(&hex_encode(&data)).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn hex_round_trip_empty() {
        assert_eq!(hex_encode(&[]), "");
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn hex_decode_rejects_odd_length() {
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("a").is_none());
    }

    #[test]
    fn hex_decode_rejects_non_hex() {
        assert!(hex_decode("zz").is_none());
        assert!(hex_decode("0g").is_none());
    }

    // JSON conversion tests are only available when serde_json is present.
    #[cfg(any(feature = "serde", target_arch = "wasm32", target_os = "android"))]
    mod json_tests {
        use super::super::*;
        use std::collections::HashMap;

        #[test]
        fn value_json_round_trip_null() {
            let s = value_to_json_string(&Value::Null).unwrap();
            assert_eq!(json_string_to_value(&s).unwrap(), Value::Null);
        }

        #[test]
        fn value_json_round_trip_bool() {
            for b in [true, false] {
                let v = Value::Bool(b);
                let s = value_to_json_string(&v).unwrap();
                assert_eq!(json_string_to_value(&s).unwrap(), v);
            }
        }

        #[test]
        fn value_json_round_trip_int() {
            for n in [0i64, 1, -1, 42, i64::MAX, i64::MIN] {
                let v = Value::Int(n);
                let s = value_to_json_string(&v).unwrap();
                assert_eq!(json_string_to_value(&s).unwrap(), v);
            }
        }

        #[test]
        fn value_json_round_trip_float() {
            let v = Value::Float(3.25);
            let s = value_to_json_string(&v).unwrap();
            match json_string_to_value(&s).unwrap() {
                Value::Float(n) if (n - 3.25).abs() < 1e-12 => {}
                other => panic!("unexpected float round-trip: {other:?}"),
            }
        }

        #[test]
        fn value_json_round_trip_string() {
            let v = Value::String("hello world".into());
            let s = value_to_json_string(&v).unwrap();
            assert_eq!(json_string_to_value(&s).unwrap(), v);
        }

        #[test]
        fn value_json_round_trip_data_sentinel() {
            let v = Value::Data(vec![0xDE, 0xAD, 0xBE, 0xEF]);
            let s = value_to_json_string(&v).unwrap();
            assert!(s.contains(DATA_SENTINEL));
            assert!(s.contains("deadbeef"));
            assert_eq!(json_string_to_value(&s).unwrap(), v);
        }

        #[test]
        fn value_json_round_trip_empty_data() {
            let v = Value::Data(vec![]);
            let s = value_to_json_string(&v).unwrap();
            assert_eq!(json_string_to_value(&s).unwrap(), v);
        }

        #[test]
        fn value_json_round_trip_array() {
            let v = Value::Array(vec![
                Value::Int(1),
                Value::String("two".into()),
                Value::Bool(true),
                Value::Data(vec![0xAB, 0xCD]),
            ]);
            let s = value_to_json_string(&v).unwrap();
            assert_eq!(json_string_to_value(&s).unwrap(), v);
        }

        #[test]
        fn value_json_round_trip_object() {
            let mut map = HashMap::new();
            map.insert("name".into(), Value::String("Brandon".into()));
            map.insert("age".into(), Value::Int(42));
            map.insert("premium".into(), Value::Bool(true));
            map.insert("payload".into(), Value::Data(vec![1, 2, 3]));

            let v = Value::Object(map);
            let s = value_to_json_string(&v).unwrap();
            assert_eq!(json_string_to_value(&s).unwrap(), v);
        }

        #[test]
        fn value_json_round_trip_nested() {
            let mut inner = HashMap::new();
            inner.insert("x".into(), Value::Int(1));
            inner.insert("bytes".into(), Value::Data(vec![0xFF, 0x00, 0xAA]));

            let mut outer = HashMap::new();
            outer.insert(
                "nums".into(),
                Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            );
            outer.insert("inner".into(), Value::Object(inner));

            let v = Value::Object(outer);
            let s = value_to_json_string(&v).unwrap();
            assert_eq!(json_string_to_value(&s).unwrap(), v);
        }

        #[test]
        fn json_to_value_rejects_invalid_data_hex() {
            let s = format!("{{\"{DATA_SENTINEL}\":\"zz\"}}");
            let err = json_string_to_value(&s).unwrap_err();
            match err {
                crate::error::Error::Parse(msg) => assert!(msg.contains("invalid hex")),
                other => panic!("expected Parse error, got {other:?}"),
            }
        }
    }
}
