use std::collections::HashMap;
use std::fmt;

use crate::Error;

/// Represents a persistable value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Data(Vec<u8>),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}

impl Value {
    /// Returns a static string describing the variant.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "Null",
            Value::Bool(_) => "Bool",
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::String(_) => "String",
            Value::Data(_) => "Data",
            Value::Array(_) => "Array",
            Value::Object(_) => "Object",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(n) => write!(f, "{n}"),
            Value::String(s) => write!(f, "{s}"),
            Value::Data(bytes) => write!(f, "<{} bytes>", bytes.len()),
            Value::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Value::Object(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

// --- From<T> for Value ---

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v as i64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v as f64)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Data(v)
    }
}

impl From<&[u8]> for Value {
    fn from(v: &[u8]) -> Self {
        Value::Data(v.to_vec())
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::Array(v)
    }
}

impl From<HashMap<String, Value>> for Value {
    fn from(v: HashMap<String, Value>) -> Self {
        Value::Object(v)
    }
}

// --- TryFrom<Value> for T (strict, type mismatch = error) ---

impl TryFrom<Value> for bool {
    type Error = Error;
    fn try_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Bool(b) => Ok(b),
            other => Err(Error::TypeMismatch {
                expected: "Bool",
                actual: other.type_name(),
            }),
        }
    }
}

impl TryFrom<Value> for i64 {
    type Error = Error;
    fn try_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Int(n) => Ok(n),
            other => Err(Error::TypeMismatch {
                expected: "Int",
                actual: other.type_name(),
            }),
        }
    }
}

impl TryFrom<Value> for f64 {
    type Error = Error;
    fn try_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Float(n) => Ok(n),
            other => Err(Error::TypeMismatch {
                expected: "Float",
                actual: other.type_name(),
            }),
        }
    }
}

impl TryFrom<Value> for String {
    type Error = Error;
    fn try_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::String(s) => Ok(s),
            other => Err(Error::TypeMismatch {
                expected: "String",
                actual: other.type_name(),
            }),
        }
    }
}

impl TryFrom<Value> for Vec<u8> {
    type Error = Error;
    fn try_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Data(d) => Ok(d),
            other => Err(Error::TypeMismatch {
                expected: "Data",
                actual: other.type_name(),
            }),
        }
    }
}

impl TryFrom<Value> for Vec<Value> {
    type Error = Error;
    fn try_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Array(a) => Ok(a),
            other => Err(Error::TypeMismatch {
                expected: "Array",
                actual: other.type_name(),
            }),
        }
    }
}

impl TryFrom<Value> for HashMap<String, Value> {
    type Error = Error;
    fn try_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Object(o) => Ok(o),
            other => Err(Error::TypeMismatch {
                expected: "Object",
                actual: other.type_name(),
            }),
        }
    }
}

// --- CoerceFrom<Value> (best-effort conversion) ---

/// Best-effort type coercion from Value.
///
/// Unlike TryFrom, CoerceFrom will attempt reasonable conversions:
/// any scalar to String, String to numeric (via parse), Int <-> Float,
/// Bool <-> Int (true=1/false=0), Data <-> String (UTF-8).
///
/// Nonsensical conversions (Array -> Int, Object -> Bool, etc.) still error.
pub trait CoerceFrom<T>: Sized {
    fn coerce_from(value: T) -> Result<Self, Error>;
}

impl CoerceFrom<Value> for bool {
    fn coerce_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Bool(b) => Ok(b),
            Value::Int(0) => Ok(false),
            Value::Int(1) => Ok(true),
            Value::Int(_) => Err(Error::CoercionFailed {
                from: "Int",
                to: "Bool",
                reason: "only 0 and 1 can coerce to Bool".into(),
            }),
            Value::String(ref s) => match s.to_lowercase().as_str() {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                _ => Err(Error::CoercionFailed {
                    from: "String",
                    to: "Bool",
                    reason: format!("cannot parse '{s}' as Bool"),
                }),
            },
            other => Err(Error::CoercionFailed {
                from: other.type_name(),
                to: "Bool",
                reason: "no reasonable coercion exists".into(),
            }),
        }
    }
}

impl CoerceFrom<Value> for i64 {
    fn coerce_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Int(n) => Ok(n),
            Value::Float(n) => Ok(n as i64),
            Value::Bool(b) => Ok(if b { 1 } else { 0 }),
            Value::String(ref s) => s.parse::<i64>().map_err(|_| Error::CoercionFailed {
                from: "String",
                to: "Int",
                reason: format!("cannot parse '{s}' as Int"),
            }),
            other => Err(Error::CoercionFailed {
                from: other.type_name(),
                to: "Int",
                reason: "no reasonable coercion exists".into(),
            }),
        }
    }
}

impl CoerceFrom<Value> for f64 {
    fn coerce_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Float(n) => Ok(n),
            Value::Int(n) => Ok(n as f64),
            Value::Bool(b) => Ok(if b { 1.0 } else { 0.0 }),
            Value::String(ref s) => s.parse::<f64>().map_err(|_| Error::CoercionFailed {
                from: "String",
                to: "Float",
                reason: format!("cannot parse '{s}' as Float"),
            }),
            other => Err(Error::CoercionFailed {
                from: other.type_name(),
                to: "Float",
                reason: "no reasonable coercion exists".into(),
            }),
        }
    }
}

impl CoerceFrom<Value> for String {
    fn coerce_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::String(s) => Ok(s),
            Value::Int(n) => Ok(n.to_string()),
            Value::Float(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            Value::Null => Ok("null".into()),
            Value::Data(ref bytes) => {
                std::string::String::from_utf8(bytes.clone()).map_err(|_| Error::CoercionFailed {
                    from: "Data",
                    to: "String",
                    reason: "data is not valid UTF-8".into(),
                })
            }
            other => Err(Error::CoercionFailed {
                from: other.type_name(),
                to: "String",
                reason: "no reasonable coercion exists".into(),
            }),
        }
    }
}

impl CoerceFrom<Value> for Vec<u8> {
    fn coerce_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Data(d) => Ok(d),
            Value::String(s) => Ok(s.into_bytes()),
            other => Err(Error::CoercionFailed {
                from: other.type_name(),
                to: "Data",
                reason: "no reasonable coercion exists".into(),
            }),
        }
    }
}

impl CoerceFrom<Value> for Vec<Value> {
    fn coerce_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Array(a) => Ok(a),
            other => Err(Error::CoercionFailed {
                from: other.type_name(),
                to: "Array",
                reason: "no reasonable coercion exists".into(),
            }),
        }
    }
}

impl CoerceFrom<Value> for HashMap<String, Value> {
    fn coerce_from(v: Value) -> Result<Self, Error> {
        match v {
            Value::Object(o) => Ok(o),
            other => Err(Error::CoercionFailed {
                from: other.type_name(),
                to: "Object",
                reason: "no reasonable coercion exists".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- From<T> for Value ---

    #[test]
    fn from_bool() {
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from(false), Value::Bool(false));
    }

    #[test]
    fn from_i64() {
        assert_eq!(Value::from(42i64), Value::Int(42));
        assert_eq!(Value::from(-1i64), Value::Int(-1));
        assert_eq!(Value::from(0i64), Value::Int(0));
    }

    #[test]
    fn from_i32() {
        assert_eq!(Value::from(42i32), Value::Int(42));
    }

    #[test]
    fn from_f64() {
        assert_eq!(Value::from(3.14f64), Value::Float(3.14));
        assert_eq!(Value::from(0.0f64), Value::Float(0.0));
    }

    #[test]
    fn from_f32() {
        assert_eq!(Value::from(3.14f32), Value::Float(3.14f32 as f64));
    }

    #[test]
    fn from_string() {
        assert_eq!(
            Value::from("hello".to_string()),
            Value::String("hello".into())
        );
    }

    #[test]
    fn from_str() {
        assert_eq!(Value::from("hello"), Value::String("hello".into()));
    }

    #[test]
    fn from_vec_u8() {
        assert_eq!(Value::from(vec![1u8, 2, 3]), Value::Data(vec![1, 2, 3]));
    }

    #[test]
    fn from_slice_u8() {
        let bytes: &[u8] = &[1, 2, 3];
        assert_eq!(Value::from(bytes), Value::Data(vec![1, 2, 3]));
    }

    #[test]
    fn from_vec_value() {
        let arr = vec![Value::Int(1), Value::Bool(true)];
        assert_eq!(
            Value::from(arr.clone()),
            Value::Array(vec![Value::Int(1), Value::Bool(true)])
        );
    }

    #[test]
    fn from_hashmap() {
        let mut map = HashMap::new();
        map.insert("key".into(), Value::Int(42));
        let val = Value::from(map.clone());
        assert_eq!(val, Value::Object(map));
    }

    // --- Display ---

    #[test]
    fn display_scalars() {
        assert_eq!(Value::Null.to_string(), "null");
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Int(42).to_string(), "42");
        assert_eq!(Value::Float(3.14).to_string(), "3.14");
        assert_eq!(Value::String("hi".into()).to_string(), "hi");
        assert_eq!(Value::Data(vec![0; 5]).to_string(), "<5 bytes>");
    }

    // --- TryFrom<Value> (strict) ---

    #[test]
    fn try_from_bool_strict() {
        assert_eq!(bool::try_from(Value::Bool(true)).unwrap(), true);
        assert!(bool::try_from(Value::Int(1)).is_err());
    }

    #[test]
    fn try_from_i64_strict() {
        assert_eq!(i64::try_from(Value::Int(42)).unwrap(), 42);
        assert!(i64::try_from(Value::Float(42.0)).is_err());
        assert!(i64::try_from(Value::String("42".into())).is_err());
    }

    #[test]
    fn try_from_f64_strict() {
        assert_eq!(f64::try_from(Value::Float(3.14)).unwrap(), 3.14);
        assert!(f64::try_from(Value::Int(3)).is_err());
    }

    #[test]
    fn try_from_string_strict() {
        assert_eq!(
            String::try_from(Value::String("hello".into())).unwrap(),
            "hello"
        );
        assert!(String::try_from(Value::Int(42)).is_err());
    }

    #[test]
    fn try_from_data_strict() {
        assert_eq!(
            Vec::<u8>::try_from(Value::Data(vec![1, 2, 3])).unwrap(),
            vec![1, 2, 3]
        );
        assert!(Vec::<u8>::try_from(Value::String("hello".into())).is_err());
    }

    #[test]
    fn try_from_array_strict() {
        let arr = vec![Value::Int(1)];
        assert_eq!(
            Vec::<Value>::try_from(Value::Array(arr.clone())).unwrap(),
            arr
        );
        assert!(Vec::<Value>::try_from(Value::Int(1)).is_err());
    }

    #[test]
    fn try_from_object_strict() {
        let mut map = HashMap::new();
        map.insert("k".into(), Value::Int(1));
        assert_eq!(
            HashMap::<String, Value>::try_from(Value::Object(map.clone())).unwrap(),
            map
        );
        assert!(HashMap::<String, Value>::try_from(Value::Array(vec![])).is_err());
    }

    // --- CoerceFrom<Value> (best-effort) ---

    #[test]
    fn coerce_bool_from_int() {
        assert_eq!(bool::coerce_from(Value::Int(0)).unwrap(), false);
        assert_eq!(bool::coerce_from(Value::Int(1)).unwrap(), true);
        assert!(bool::coerce_from(Value::Int(42)).is_err());
    }

    #[test]
    fn coerce_bool_from_string() {
        assert_eq!(
            bool::coerce_from(Value::String("true".into())).unwrap(),
            true
        );
        assert_eq!(
            bool::coerce_from(Value::String("false".into())).unwrap(),
            false
        );
        assert_eq!(
            bool::coerce_from(Value::String("TRUE".into())).unwrap(),
            true
        );
        assert_eq!(bool::coerce_from(Value::String("1".into())).unwrap(), true);
        assert_eq!(bool::coerce_from(Value::String("0".into())).unwrap(), false);
        assert!(bool::coerce_from(Value::String("maybe".into())).is_err());
    }

    #[test]
    fn coerce_bool_from_nonsensical() {
        assert!(bool::coerce_from(Value::Array(vec![])).is_err());
        assert!(bool::coerce_from(Value::Null).is_err());
    }

    #[test]
    fn coerce_i64_from_float() {
        assert_eq!(i64::coerce_from(Value::Float(42.9)).unwrap(), 42);
    }

    #[test]
    fn coerce_i64_from_bool() {
        assert_eq!(i64::coerce_from(Value::Bool(true)).unwrap(), 1);
        assert_eq!(i64::coerce_from(Value::Bool(false)).unwrap(), 0);
    }

    #[test]
    fn coerce_i64_from_string() {
        assert_eq!(i64::coerce_from(Value::String("42".into())).unwrap(), 42);
        assert_eq!(i64::coerce_from(Value::String("-7".into())).unwrap(), -7);
        assert!(i64::coerce_from(Value::String("nope".into())).is_err());
    }

    #[test]
    fn coerce_f64_from_int() {
        assert_eq!(f64::coerce_from(Value::Int(42)).unwrap(), 42.0);
    }

    #[test]
    fn coerce_f64_from_bool() {
        assert_eq!(f64::coerce_from(Value::Bool(true)).unwrap(), 1.0);
        assert_eq!(f64::coerce_from(Value::Bool(false)).unwrap(), 0.0);
    }

    #[test]
    fn coerce_f64_from_string() {
        assert_eq!(
            f64::coerce_from(Value::String("3.14".into())).unwrap(),
            3.14
        );
        assert!(f64::coerce_from(Value::String("nope".into())).is_err());
    }

    #[test]
    fn coerce_string_from_scalars() {
        assert_eq!(String::coerce_from(Value::Int(42)).unwrap(), "42");
        assert_eq!(String::coerce_from(Value::Float(3.14)).unwrap(), "3.14");
        assert_eq!(String::coerce_from(Value::Bool(true)).unwrap(), "true");
        assert_eq!(String::coerce_from(Value::Null).unwrap(), "null");
    }

    #[test]
    fn coerce_string_from_data_utf8() {
        let bytes = "hello".as_bytes().to_vec();
        assert_eq!(String::coerce_from(Value::Data(bytes)).unwrap(), "hello");
    }

    #[test]
    fn coerce_string_from_data_invalid_utf8() {
        let bytes = vec![0xFF, 0xFE];
        assert!(String::coerce_from(Value::Data(bytes)).is_err());
    }

    #[test]
    fn coerce_string_from_array_fails() {
        assert!(String::coerce_from(Value::Array(vec![])).is_err());
    }

    #[test]
    fn coerce_data_from_string() {
        assert_eq!(
            Vec::<u8>::coerce_from(Value::String("hello".into())).unwrap(),
            b"hello".to_vec()
        );
    }

    #[test]
    fn coerce_array_only_from_array() {
        let arr = vec![Value::Int(1)];
        assert_eq!(
            Vec::<Value>::coerce_from(Value::Array(arr.clone())).unwrap(),
            arr
        );
        assert!(Vec::<Value>::coerce_from(Value::Int(1)).is_err());
    }

    #[test]
    fn coerce_object_only_from_object() {
        let mut map = HashMap::new();
        map.insert("k".into(), Value::Int(1));
        assert_eq!(
            HashMap::<String, Value>::coerce_from(Value::Object(map.clone())).unwrap(),
            map
        );
        assert!(HashMap::<String, Value>::coerce_from(Value::Int(1)).is_err());
    }

    // --- Edge cases ---

    #[test]
    fn value_type_name() {
        assert_eq!(Value::Null.type_name(), "Null");
        assert_eq!(Value::Bool(true).type_name(), "Bool");
        assert_eq!(Value::Int(0).type_name(), "Int");
        assert_eq!(Value::Float(0.0).type_name(), "Float");
        assert_eq!(Value::String("".into()).type_name(), "String");
        assert_eq!(Value::Data(vec![]).type_name(), "Data");
        assert_eq!(Value::Array(vec![]).type_name(), "Array");
        assert_eq!(Value::Object(HashMap::new()).type_name(), "Object");
    }

    #[test]
    fn from_empty_containers() {
        assert_eq!(Value::from(Vec::<u8>::new()), Value::Data(vec![]));
        assert_eq!(Value::from(Vec::<Value>::new()), Value::Array(vec![]));
        assert_eq!(
            Value::from(HashMap::<String, Value>::new()),
            Value::Object(HashMap::new())
        );
    }

    #[test]
    fn from_empty_string() {
        assert_eq!(Value::from(""), Value::String("".into()));
    }

    #[test]
    fn from_large_int() {
        let big = i64::MAX;
        assert_eq!(Value::from(big), Value::Int(big));
        assert_eq!(i64::try_from(Value::Int(big)).unwrap(), big);
    }

    #[test]
    fn nested_structure_round_trip() {
        let mut inner = HashMap::new();
        inner.insert("x".into(), Value::Int(1));

        let nested = Value::Object({
            let mut m = HashMap::new();
            m.insert(
                "arr".into(),
                Value::Array(vec![
                    Value::Object(inner.clone()),
                    Value::String("hello".into()),
                    Value::Null,
                ]),
            );
            m.insert("flag".into(), Value::Bool(true));
            m
        });

        // Verify clone equals original
        assert_eq!(nested, nested.clone());
    }
}
