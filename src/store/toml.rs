use std::collections::HashMap;

use crate::error::Error;
use crate::store::file_backed::{FileBackedStore, Format};
use crate::value::Value;

/// TOML serialization format.
pub struct TomlFormat;

/// TOML file-backed store.
pub type TomlFileStore = FileBackedStore<TomlFormat>;

// TOML has no binary type. Data is stored as a string with a sentinel prefix.
const DATA_SENTINEL_PREFIX: &str = "$persist:data:";

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Convert our Value into a toml::Value. Nulls return None so callers can skip
/// them (TOML has no null representation).
fn value_to_toml(v: &Value) -> Option<toml::Value> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        Value::Int(n) => Some(toml::Value::Integer(*n)),
        Value::Float(n) => Some(toml::Value::Float(*n)),
        Value::String(s) => Some(toml::Value::String(s.clone())),
        Value::Data(bytes) => Some(toml::Value::String(format!(
            "{}{}",
            DATA_SENTINEL_PREFIX,
            hex_encode(bytes)
        ))),
        Value::Array(arr) => {
            let items: Vec<toml::Value> = arr.iter().filter_map(value_to_toml).collect();
            Some(toml::Value::Array(items))
        }
        Value::Object(obj) => {
            let mut table = toml::value::Table::new();
            for (k, v) in obj {
                if let Some(tv) = value_to_toml(v) {
                    table.insert(k.clone(), tv);
                }
            }
            Some(toml::Value::Table(table))
        }
    }
}

fn toml_to_value(v: toml::Value) -> Value {
    match v {
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Integer(n) => Value::Int(n),
        toml::Value::Float(n) => Value::Float(n),
        toml::Value::String(s) => {
            if let Some(hex) = s.strip_prefix(DATA_SENTINEL_PREFIX) {
                if let Some(bytes) = hex_decode(hex) {
                    return Value::Data(bytes);
                }
            }
            Value::String(s)
        }
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(arr) => Value::Array(arr.into_iter().map(toml_to_value).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .into_iter()
                .map(|(k, v)| (k, toml_to_value(v)))
                .collect(),
        ),
    }
}

impl Format for TomlFormat {
    fn serialize(data: &HashMap<String, Value>) -> Result<String, Error> {
        let mut table = toml::value::Table::new();
        for (k, v) in data {
            if let Some(tv) = value_to_toml(v) {
                table.insert(k.clone(), tv);
            }
        }
        toml::to_string_pretty(&table).map_err(|e| Error::Parse(e.to_string()))
    }

    fn deserialize(text: &str) -> Result<HashMap<String, Value>, Error> {
        let table: toml::Table = toml::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
        Ok(table
            .into_iter()
            .map(|(k, v)| (k, toml_to_value(v)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("persist_tests_toml");
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
        let serialized = TomlFormat::serialize(&data).unwrap();
        let deserialized = TomlFormat::deserialize(&serialized).unwrap();
        assert_eq!(
            data, deserialized,
            "round-trip failed.\nSerialized:\n{serialized}"
        );
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
        data.insert("pi".into(), Value::Float(3.25));
        data.insert("neg".into(), Value::Float(-1.5));
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
        data.insert("empty_data".into(), Value::Data(vec![]));
        round_trip(data);
    }

    #[test]
    fn round_trip_array() {
        let mut data = HashMap::new();
        data.insert("empty".into(), Value::Array(vec![]));
        data.insert(
            "ints".into(),
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
        data.insert(
            "mixed".into(),
            Value::Array(vec![
                Value::Int(1),
                Value::String("two".into()),
                Value::Bool(true),
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
        data.insert("point".into(), Value::Object(inner));
        round_trip(data);
    }

    #[test]
    fn round_trip_nested_objects() {
        let mut db = HashMap::new();
        db.insert("host".into(), Value::String("localhost".into()));
        db.insert("port".into(), Value::Int(5432));

        let mut server = HashMap::new();
        server.insert("name".into(), Value::String("main".into()));
        server.insert("db".into(), Value::Object(db));

        let mut data = HashMap::new();
        data.insert("server".into(), Value::Object(server));
        round_trip(data);
    }

    #[test]
    fn round_trip_data_in_nested() {
        let mut tls = HashMap::new();
        tls.insert("cert".into(), Value::Data(vec![0xDE, 0xAD]));

        let mut data = HashMap::new();
        data.insert("tls".into(), Value::Object(tls));
        data.insert(
            "keys".into(),
            Value::Array(vec![Value::Data(vec![1, 2]), Value::Data(vec![3, 4])]),
        );
        round_trip(data);
    }

    #[test]
    fn round_trip_inline_table_in_array() {
        let mut item = HashMap::new();
        item.insert("name".into(), Value::String("widget".into()));
        item.insert("count".into(), Value::Int(5));

        let mut data = HashMap::new();
        data.insert("items".into(), Value::Array(vec![Value::Object(item)]));
        round_trip(data);
    }

    // --- Null handling ---

    #[test]
    fn null_omitted_via_store_set() {
        let path = temp_path("null_omit.toml");
        cleanup(&path);

        let store = TomlFileStore::new(&path);
        store.set("key", Value::String("value".into())).unwrap();
        store.set("key", Value::Null).unwrap();
        assert_eq!(store.get("key").unwrap(), None);

        cleanup(&path);
    }

    // --- Parse edge cases ---

    #[test]
    fn parse_empty() {
        let result = TomlFormat::deserialize("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_comments() {
        let toml = "# comment\nkey = \"value\" # inline comment\n";
        let result = TomlFormat::deserialize(toml).unwrap();
        assert_eq!(result.get("key"), Some(&Value::String("value".into())));
    }

    #[test]
    fn parse_underscores_in_numbers() {
        let toml = "big = 1_000_000\n";
        let result = TomlFormat::deserialize(toml).unwrap();
        assert_eq!(result.get("big"), Some(&Value::Int(1_000_000)));
    }

    #[test]
    fn parse_malformed_errors() {
        assert!(TomlFormat::deserialize("= 1").is_err());
        assert!(TomlFormat::deserialize("key =").is_err());
    }

    // --- Full file store integration ---

    #[test]
    fn toml_file_store_basic() {
        let path = temp_path("basic.toml");
        cleanup(&path);

        let store = TomlFileStore::new(&path);
        store.set("name", Value::String("Brandon".into())).unwrap();
        store.set("age", Value::Int(42)).unwrap();

        assert_eq!(
            store.get("name").unwrap(),
            Some(Value::String("Brandon".into()))
        );
        assert_eq!(store.get("age").unwrap(), Some(Value::Int(42)));

        cleanup(&path);
    }

    #[test]
    fn toml_file_store_persists_across_instances() {
        let path = temp_path("persist.toml");
        cleanup(&path);

        {
            let store = TomlFileStore::new(&path);
            store.set("key", Value::String("value".into())).unwrap();
        }
        {
            let store = TomlFileStore::new(&path);
            assert_eq!(
                store.get("key").unwrap(),
                Some(Value::String("value".into()))
            );
        }

        cleanup(&path);
    }

    #[test]
    fn toml_file_store_all_scalar_types() {
        let path = temp_path("all_scalars.toml");
        cleanup(&path);

        let store = TomlFileStore::new(&path);
        store.set("bool", Value::Bool(true)).unwrap();
        store.set("int", Value::Int(42)).unwrap();
        store.set("float", Value::Float(3.25)).unwrap();
        store.set("string", Value::String("hello".into())).unwrap();
        store.set("data", Value::Data(vec![1, 2, 3])).unwrap();

        drop(store);
        let store = TomlFileStore::new(&path);
        assert_eq!(store.get("bool").unwrap(), Some(Value::Bool(true)));
        assert_eq!(store.get("int").unwrap(), Some(Value::Int(42)));
        assert_eq!(store.get("float").unwrap(), Some(Value::Float(3.25)));
        assert_eq!(
            store.get("string").unwrap(),
            Some(Value::String("hello".into()))
        );
        assert_eq!(store.get("data").unwrap(), Some(Value::Data(vec![1, 2, 3])));

        cleanup(&path);
    }
}
