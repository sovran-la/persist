use std::collections::HashMap;
use std::fmt;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::convert::{hex_decode, hex_encode, DATA_SENTINEL};
use crate::value::Value;

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Int(n) => serializer.serialize_i64(*n),
            Value::Float(n) => serializer.serialize_f64(*n),
            Value::String(s) => serializer.serialize_str(s),
            Value::Data(bytes) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(DATA_SENTINEL, &hex_encode(bytes))?;
                map.end()
            }
            Value::Array(arr) => {
                let mut seq = serializer.serialize_seq(Some(arr.len()))?;
                for v in arr {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Value::Object(obj) => {
                let mut map = serializer.serialize_map(Some(obj.len()))?;
                for (k, v) in obj {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any valid persist Value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Int(v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        if v <= i64::MAX as u64 {
            Ok(Value::Int(v as i64))
        } else {
            Ok(Value::Float(v as f64))
        }
    }

    fn visit_i128<E>(self, v: i128) -> Result<Value, E> {
        if v >= i64::MIN as i128 && v <= i64::MAX as i128 {
            Ok(Value::Int(v as i64))
        } else {
            Ok(Value::Float(v as f64))
        }
    }

    fn visit_u128<E>(self, v: u128) -> Result<Value, E> {
        if v <= i64::MAX as u128 {
            Ok(Value::Int(v as i64))
        } else {
            Ok(Value::Float(v as f64))
        }
    }

    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Value, E> {
        Ok(Value::Data(v.to_vec()))
    }

    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Value, E> {
        Ok(Value::Data(v))
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        Value::deserialize(d)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut arr = Vec::new();
        while let Some(v) = seq.next_element()? {
            arr.push(v);
        }
        Ok(Value::Array(arr))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut obj: HashMap<String, Value> = HashMap::new();
        while let Some((k, v)) = map.next_entry::<String, Value>()? {
            obj.insert(k, v);
        }
        // Check for Data sentinel: single-key map with DATA_SENTINEL key
        // pointing at a hex string.
        if obj.len() == 1 {
            if let Some(Value::String(hex)) = obj.get(DATA_SENTINEL) {
                if let Some(bytes) = hex_decode(hex) {
                    return Ok(Value::Data(bytes));
                }
            }
        }
        Ok(Value::Object(obj))
    }
}

// --- Conversions with serde_json::Value ---

impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    if u <= i64::MAX as u64 {
                        Value::Int(u as i64)
                    } else {
                        Value::Float(u as f64)
                    }
                } else if let Some(f) = n.as_f64() {
                    Value::Float(f)
                } else {
                    Value::Null
                }
            }
            serde_json::Value::String(s) => Value::String(s),
            serde_json::Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Value::from).collect())
            }
            serde_json::Value::Object(obj) => {
                // Check for data sentinel
                if obj.len() == 1 {
                    if let Some(serde_json::Value::String(hex)) = obj.get(DATA_SENTINEL) {
                        if let Some(bytes) = hex_decode(hex) {
                            return Value::Data(bytes);
                        }
                    }
                }
                Value::Object(obj.into_iter().map(|(k, v)| (k, Value::from(v))).collect())
            }
        }
    }
}

impl From<Value> for serde_json::Value {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(b),
            Value::Int(n) => serde_json::Value::Number(n.into()),
            Value::Float(n) => serde_json::Number::from_f64(n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::String(s) => serde_json::Value::String(s),
            Value::Data(bytes) => {
                let mut map = serde_json::Map::new();
                map.insert(
                    DATA_SENTINEL.to_owned(),
                    serde_json::Value::String(hex_encode(&bytes)),
                );
                serde_json::Value::Object(map)
            }
            Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(serde_json::Value::from).collect())
            }
            Value::Object(obj) => serde_json::Value::Object(
                obj.into_iter()
                    .map(|(k, v)| (k, serde_json::Value::from(v)))
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- hex helpers ---

    #[test]
    fn hex_round_trip_bytes() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0],
            vec![255],
            vec![0, 1, 127, 128, 255],
            vec![0xDE, 0xAD, 0xBE, 0xEF],
        ];
        for bytes in cases {
            let encoded = hex_encode(&bytes);
            let decoded = hex_decode(&encoded).unwrap();
            assert_eq!(bytes, decoded);
        }
    }

    #[test]
    fn hex_decode_invalid() {
        assert!(hex_decode("0").is_none()); // odd length
        assert!(hex_decode("zz").is_none()); // invalid chars
    }

    // --- Serialize/Deserialize via serde_json ---

    fn json_round_trip(v: Value) {
        let s = serde_json::to_string(&v).unwrap();
        let out: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v, out, "round trip mismatch.\nJSON: {s}");
    }

    #[test]
    fn serde_null() {
        json_round_trip(Value::Null);
    }

    #[test]
    fn serde_bool() {
        json_round_trip(Value::Bool(true));
        json_round_trip(Value::Bool(false));
    }

    #[test]
    fn serde_int() {
        json_round_trip(Value::Int(0));
        json_round_trip(Value::Int(42));
        json_round_trip(Value::Int(-7));
        json_round_trip(Value::Int(i64::MAX));
        json_round_trip(Value::Int(i64::MIN));
    }

    #[test]
    fn serde_float() {
        json_round_trip(Value::Float(3.14));
        json_round_trip(Value::Float(0.0));
        json_round_trip(Value::Float(-1.5));
    }

    #[test]
    fn serde_string() {
        json_round_trip(Value::String("hello".into()));
        json_round_trip(Value::String("".into()));
        json_round_trip(Value::String("line1\nline2\ttab \"quoted\"".into()));
    }

    #[test]
    fn serde_data() {
        json_round_trip(Value::Data(vec![0, 1, 127, 255]));
        json_round_trip(Value::Data(vec![]));
        json_round_trip(Value::Data(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    }

    #[test]
    fn serde_array() {
        json_round_trip(Value::Array(vec![]));
        json_round_trip(Value::Array(vec![
            Value::Int(1),
            Value::String("two".into()),
            Value::Bool(true),
            Value::Null,
        ]));
    }

    #[test]
    fn serde_object() {
        let mut map = HashMap::new();
        map.insert("x".into(), Value::Int(1));
        map.insert("y".into(), Value::Int(2));
        json_round_trip(Value::Object(map));
        json_round_trip(Value::Object(HashMap::new()));
    }

    #[test]
    fn serde_nested() {
        let mut inner = HashMap::new();
        inner.insert("cert".into(), Value::Data(vec![0xDE, 0xAD]));

        let mut data = HashMap::new();
        data.insert("tls".into(), Value::Object(inner));
        data.insert(
            "keys".into(),
            Value::Array(vec![Value::Data(vec![1, 2]), Value::Data(vec![3, 4])]),
        );
        json_round_trip(Value::Object(data));
    }

    // --- serde_json::Value conversions ---

    #[test]
    fn from_serde_json_primitives() {
        assert_eq!(Value::from(serde_json::Value::Null), Value::Null);
        assert_eq!(
            Value::from(serde_json::Value::Bool(true)),
            Value::Bool(true)
        );
        assert_eq!(Value::from(serde_json::json!(42)), Value::Int(42));
        assert_eq!(Value::from(serde_json::json!(3.14)), Value::Float(3.14));
        assert_eq!(
            Value::from(serde_json::json!("hello")),
            Value::String("hello".into())
        );
    }

    #[test]
    fn into_serde_json_primitives() {
        assert_eq!(
            serde_json::Value::from(Value::Null),
            serde_json::Value::Null
        );
        assert_eq!(
            serde_json::Value::from(Value::Bool(true)),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            serde_json::Value::from(Value::Int(42)),
            serde_json::json!(42)
        );
        assert_eq!(
            serde_json::Value::from(Value::Float(3.14)),
            serde_json::json!(3.14)
        );
        assert_eq!(
            serde_json::Value::from(Value::String("hello".into())),
            serde_json::json!("hello")
        );
    }

    #[test]
    fn from_serde_json_array() {
        let j = serde_json::json!([1, "two", true, null]);
        assert_eq!(
            Value::from(j),
            Value::Array(vec![
                Value::Int(1),
                Value::String("two".into()),
                Value::Bool(true),
                Value::Null,
            ])
        );
    }

    #[test]
    fn into_serde_json_array() {
        let v = Value::Array(vec![
            Value::Int(1),
            Value::String("two".into()),
            Value::Bool(true),
            Value::Null,
        ]);
        assert_eq!(
            serde_json::Value::from(v),
            serde_json::json!([1, "two", true, null])
        );
    }

    #[test]
    fn from_serde_json_object() {
        let j = serde_json::json!({"x": 1, "y": 2});
        match Value::from(j) {
            Value::Object(map) => {
                assert_eq!(map.get("x"), Some(&Value::Int(1)));
                assert_eq!(map.get("y"), Some(&Value::Int(2)));
            }
            other => panic!("expected Object, got {:?}", other),
        }
    }

    #[test]
    fn data_sentinel_from_serde_json() {
        let hex = "deadbeef";
        let j = serde_json::json!({ DATA_SENTINEL: hex });
        assert_eq!(Value::from(j), Value::Data(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    }

    #[test]
    fn data_sentinel_into_serde_json() {
        let v = Value::Data(vec![0xDE, 0xAD]);
        let j = serde_json::Value::from(v);
        assert_eq!(j, serde_json::json!({ DATA_SENTINEL: "dead" }));
    }

    #[test]
    fn serde_json_value_round_trip_all_types() {
        let mut inner = HashMap::new();
        inner.insert("flag".into(), Value::Bool(true));
        inner.insert("cert".into(), Value::Data(vec![1, 2, 3]));

        let mut root = HashMap::new();
        root.insert("name".into(), Value::String("Brandon".into()));
        root.insert("age".into(), Value::Int(42));
        root.insert("pi".into(), Value::Float(3.14));
        root.insert(
            "tags".into(),
            Value::Array(vec![Value::String("a".into()), Value::Int(1)]),
        );
        root.insert("meta".into(), Value::Object(inner));

        let original = Value::Object(root);
        let j = serde_json::Value::from(original.clone());
        let restored = Value::from(j);
        assert_eq!(original, restored);
    }
}
