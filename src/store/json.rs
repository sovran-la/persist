use std::collections::HashMap;

use crate::error::Error;
use crate::store::file_backed::{FileBackedStore, Format};
use crate::value::Value;

/// JSON serialization format.
pub struct JsonFormat;

/// JSON file-backed store.
pub type JsonFileStore = FileBackedStore<JsonFormat>;

impl Format for JsonFormat {
    fn serialize(data: &HashMap<String, Value>) -> Result<String, Error> {
        let root = serde_json::Value::from(Value::Object(data.clone()));
        let mut s = serde_json::to_string_pretty(&root)
            .map_err(|e| Error::Parse(e.to_string()))?;
        s.push('\n');
        Ok(s)
    }

    fn deserialize(text: &str) -> Result<HashMap<String, Value>, Error> {
        let json: serde_json::Value =
            serde_json::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
        match json {
            serde_json::Value::Object(obj) => Ok(obj
                .into_iter()
                .map(|(k, v)| (k, Value::from(v)))
                .collect()),
            _ => Err(Error::Parse("JSON root must be an object".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("persist_tests_json");
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let mut tmp = path.to_path_buf();
        let name = tmp
            .file_name()
            .map(|n| format!(".{}.tmp", n.to_string_lossy()))
            .unwrap_or_default();
        tmp.set_file_name(name);
        let _ = fs::remove_file(&tmp);
    }

    // --- Serialization round-trips ---

    fn round_trip(data: HashMap<String, Value>) {
        let serialized = JsonFormat::serialize(&data).unwrap();
        let deserialized = JsonFormat::deserialize(&serialized).unwrap();
        assert_eq!(
            data, deserialized,
            "round-trip failed.\nSerialized:\n{serialized}"
        );
    }

    #[test]
    fn round_trip_null_in_arrays() {
        // Null values survive in nested structures (arrays, objects).
        let mut data = HashMap::new();
        data.insert(
            "arr".into(),
            Value::Array(vec![Value::Null, Value::Int(1)]),
        );
        round_trip(data);
    }

    #[test]
    fn round_trip_bool() {
        let mut data = HashMap::new();
        data.insert("t".into(), Value::Bool(true));
        data.insert("f".into(), Value::Bool(false));
        round_trip(data);
    }

    #[test]
    fn round_trip_int() {
        let mut data = HashMap::new();
        data.insert("zero".into(), Value::Int(0));
        data.insert("pos".into(), Value::Int(42));
        data.insert("neg".into(), Value::Int(-7));
        data.insert("max".into(), Value::Int(i64::MAX));
        data.insert("min".into(), Value::Int(i64::MIN));
        round_trip(data);
    }

    #[test]
    fn round_trip_float() {
        let mut data = HashMap::new();
        data.insert("pi".into(), Value::Float(3.14));
        data.insert("neg".into(), Value::Float(-1.5));
        data.insert("tiny".into(), Value::Float(2e-3));
        round_trip(data);
    }

    #[test]
    fn round_trip_string() {
        let mut data = HashMap::new();
        data.insert("simple".into(), Value::String("hello".into()));
        data.insert("empty".into(), Value::String("".into()));
        data.insert(
            "escapes".into(),
            Value::String("line1\nline2\ttab \"quoted\" back\\slash".into()),
        );
        round_trip(data);
    }

    #[test]
    fn round_trip_data() {
        let mut data = HashMap::new();
        data.insert("payload".into(), Value::Data(vec![0, 1, 127, 255]));
        data.insert("empty".into(), Value::Data(vec![]));
        round_trip(data);
    }

    #[test]
    fn round_trip_array() {
        let mut data = HashMap::new();
        data.insert("empty".into(), Value::Array(vec![]));
        data.insert(
            "mixed".into(),
            Value::Array(vec![
                Value::Int(1),
                Value::String("two".into()),
                Value::Bool(true),
                Value::Null,
            ]),
        );
        round_trip(data);
    }

    #[test]
    fn round_trip_object() {
        let mut inner = HashMap::new();
        inner.insert("x".into(), Value::Int(1));
        inner.insert("y".into(), Value::Int(2));

        let mut data = HashMap::new();
        data.insert("empty".into(), Value::Object(HashMap::new()));
        data.insert("point".into(), Value::Object(inner));
        round_trip(data);
    }

    #[test]
    fn round_trip_nested() {
        let mut inner = HashMap::new();
        inner.insert("name".into(), Value::String("test".into()));
        inner.insert(
            "tags".into(),
            Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
        );

        let mut data = HashMap::new();
        data.insert(
            "items".into(),
            Value::Array(vec![Value::Object(inner), Value::Null]),
        );
        data.insert("count".into(), Value::Int(1));
        round_trip(data);
    }

    #[test]
    fn round_trip_data_in_nested_structures() {
        let mut inner = HashMap::new();
        inner.insert("cert".into(), Value::Data(vec![0xDE, 0xAD, 0xBE, 0xEF]));

        let mut data = HashMap::new();
        data.insert("tls".into(), Value::Object(inner));
        data.insert(
            "keys".into(),
            Value::Array(vec![
                Value::Data(vec![1, 2, 3]),
                Value::Data(vec![4, 5, 6]),
            ]),
        );
        round_trip(data);
    }

    // --- Parse edge cases ---

    #[test]
    fn parse_empty_object() {
        let result = JsonFormat::deserialize("{}").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_non_object_root_errors() {
        assert!(JsonFormat::deserialize("[1, 2, 3]").is_err());
        assert!(JsonFormat::deserialize("42").is_err());
        assert!(JsonFormat::deserialize("\"hello\"").is_err());
    }

    #[test]
    fn parse_unicode_escape() {
        let json = r#"{"smile": "\u0048\u0069"}"#;
        let result = JsonFormat::deserialize(json).unwrap();
        assert_eq!(result.get("smile"), Some(&Value::String("Hi".into())));
    }

    #[test]
    fn parse_scientific_notation() {
        let json = r#"{"big": 1.5e10, "small": 2E-3}"#;
        let result = JsonFormat::deserialize(json).unwrap();
        assert_eq!(result.get("big"), Some(&Value::Float(1.5e10)));
        assert_eq!(result.get("small"), Some(&Value::Float(2e-3)));
    }

    #[test]
    fn parse_malformed_errors() {
        assert!(JsonFormat::deserialize("{not_valid").is_err());
        assert!(JsonFormat::deserialize("{\"k\":").is_err());
    }

    // --- Full file store integration ---

    #[test]
    fn json_file_store_basic() {
        let path = temp_path("basic.json");
        cleanup(&path);

        let store = JsonFileStore::new(&path);
        store.set("name", Value::String("Brandon".into())).unwrap();
        store.set("age", Value::Int(42)).unwrap();
        store.set("premium", Value::Bool(true)).unwrap();

        assert_eq!(
            store.get("name").unwrap(),
            Some(Value::String("Brandon".into()))
        );
        assert_eq!(store.get("age").unwrap(), Some(Value::Int(42)));
        assert_eq!(store.get("premium").unwrap(), Some(Value::Bool(true)));

        cleanup(&path);
    }

    #[test]
    fn json_file_store_persists_across_instances() {
        let path = temp_path("persist.json");
        cleanup(&path);

        {
            let store = JsonFileStore::new(&path);
            store.set("key", Value::String("value".into())).unwrap();
        }
        {
            let store = JsonFileStore::new(&path);
            assert_eq!(
                store.get("key").unwrap(),
                Some(Value::String("value".into()))
            );
        }

        cleanup(&path);
    }

    #[test]
    fn json_file_store_data_round_trip() {
        let path = temp_path("data.json");
        cleanup(&path);

        let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
        {
            let store = JsonFileStore::new(&path);
            store.set("payload", Value::Data(bytes.clone())).unwrap();
        }
        {
            let store = JsonFileStore::new(&path);
            assert_eq!(store.get("payload").unwrap(), Some(Value::Data(bytes)));
        }

        cleanup(&path);
    }

    #[test]
    fn json_file_store_all_types() {
        let path = temp_path("all_types.json");
        cleanup(&path);

        let mut obj = HashMap::new();
        obj.insert("nested".into(), Value::Bool(true));

        let store = JsonFileStore::new(&path);
        store.set("bool", Value::Bool(true)).unwrap();
        store.set("int", Value::Int(42)).unwrap();
        store.set("float", Value::Float(3.14)).unwrap();
        store.set("string", Value::String("hello".into())).unwrap();
        store.set("data", Value::Data(vec![1, 2, 3])).unwrap();
        store
            .set(
                "array",
                Value::Array(vec![Value::Int(1), Value::String("two".into())]),
            )
            .unwrap();
        store.set("object", Value::Object(obj.clone())).unwrap();

        drop(store);
        let store = JsonFileStore::new(&path);
        assert_eq!(store.get("bool").unwrap(), Some(Value::Bool(true)));
        assert_eq!(store.get("int").unwrap(), Some(Value::Int(42)));
        assert_eq!(store.get("float").unwrap(), Some(Value::Float(3.14)));
        assert_eq!(
            store.get("string").unwrap(),
            Some(Value::String("hello".into()))
        );
        assert_eq!(store.get("data").unwrap(), Some(Value::Data(vec![1, 2, 3])));
        assert_eq!(
            store.get("array").unwrap(),
            Some(Value::Array(vec![
                Value::Int(1),
                Value::String("two".into())
            ]))
        );
        assert_eq!(store.get("object").unwrap(), Some(Value::Object(obj)));

        cleanup(&path);
    }
}
