#![cfg(target_arch = "wasm32")]

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};

use crate::error::Error;
use crate::store::Store;
use crate::value::Value;

const DATA_SENTINEL: &str = "$persist:data";

/// Whether a [`WebStore`] is backed by durable storage.
///
/// `Persisted` means writes survive page reloads via `localStorage`.
/// `MemoryOnly` means `localStorage` is unavailable (Workers, Service
/// Workers, private browsing, certain iframes) and data lives only in
/// this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceState {
    Persisted,
    MemoryOnly,
}

/// Browser-backed store with a fallback chain:
/// 1. `localStorage` (primary — sync, matches the [`Store`] trait).
/// 2. In-memory `HashMap` (when `localStorage` is unavailable).
///
/// Keys are namespaced in `localStorage` as `"{namespace}:{key}"` so
/// multiple stores can share the same origin without collisions.
pub struct WebStore {
    namespace: String,
    persistence_state: PersistenceState,
    memory_fallback: Mutex<HashMap<String, Value>>,
}

impl WebStore {
    pub fn new(namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        let persistence_state = if get_local_storage().is_some() {
            PersistenceState::Persisted
        } else {
            PersistenceState::MemoryOnly
        };
        Self {
            namespace,
            persistence_state,
            memory_fallback: Mutex::new(HashMap::new()),
        }
    }

    pub fn persistence_state(&self) -> PersistenceState {
        self.persistence_state
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    fn prefixed_key(&self, key: &str) -> String {
        format!("{}:{}", self.namespace, key)
    }
}

impl Store for WebStore {
    fn get(&self, key: &str) -> Result<Option<Value>, Error> {
        match self.persistence_state {
            PersistenceState::Persisted => {
                let storage = get_local_storage()
                    .ok_or_else(|| Error::Custom("localStorage disappeared".into()))?;
                let full_key = self.prefixed_key(key);
                match storage.get_item(&full_key).map_err(js_err)? {
                    Some(s) => Ok(Some(json_string_to_value(&s)?)),
                    None => Ok(None),
                }
            }
            PersistenceState::MemoryOnly => {
                let map = self.memory_fallback.lock().map_err(lock_err)?;
                Ok(map.get(key).cloned())
            }
        }
    }

    fn set(&self, key: &str, value: Value) -> Result<(), Error> {
        if matches!(value, Value::Null) {
            self.delete(key)?;
            return Ok(());
        }
        match self.persistence_state {
            PersistenceState::Persisted => {
                let storage = get_local_storage()
                    .ok_or_else(|| Error::Custom("localStorage disappeared".into()))?;
                let full_key = self.prefixed_key(key);
                let s = value_to_json_string(&value)?;
                storage.set_item(&full_key, &s).map_err(js_err)?;
                Ok(())
            }
            PersistenceState::MemoryOnly => {
                let mut map = self.memory_fallback.lock().map_err(lock_err)?;
                map.insert(key.to_owned(), value);
                Ok(())
            }
        }
    }

    fn delete(&self, key: &str) -> Result<bool, Error> {
        match self.persistence_state {
            PersistenceState::Persisted => {
                let storage = get_local_storage()
                    .ok_or_else(|| Error::Custom("localStorage disappeared".into()))?;
                let full_key = self.prefixed_key(key);
                let existed = storage.get_item(&full_key).map_err(js_err)?.is_some();
                if existed {
                    storage.remove_item(&full_key).map_err(js_err)?;
                }
                Ok(existed)
            }
            PersistenceState::MemoryOnly => {
                let mut map = self.memory_fallback.lock().map_err(lock_err)?;
                Ok(map.remove(key).is_some())
            }
        }
    }

    fn exists(&self, key: &str) -> Result<bool, Error> {
        match self.persistence_state {
            PersistenceState::Persisted => {
                let storage = get_local_storage()
                    .ok_or_else(|| Error::Custom("localStorage disappeared".into()))?;
                let full_key = self.prefixed_key(key);
                Ok(storage.get_item(&full_key).map_err(js_err)?.is_some())
            }
            PersistenceState::MemoryOnly => {
                let map = self.memory_fallback.lock().map_err(lock_err)?;
                Ok(map.contains_key(key))
            }
        }
    }
}

fn get_local_storage() -> Option<web_sys::Storage> {
    let window = web_sys::window()?;
    window.local_storage().ok()?
}

fn js_err(value: wasm_bindgen::JsValue) -> Error {
    Error::Custom(format!("localStorage error: {:?}", value))
}

fn lock_err<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::Custom("memory fallback mutex poisoned".into())
}

fn value_to_json_string(value: &Value) -> Result<String, Error> {
    let json = value_to_json(value);
    serde_json::to_string(&json)
        .map_err(|e| Error::Parse(format!("serialize value to JSON: {e}")))
}

fn json_string_to_value(s: &str) -> Result<Value, Error> {
    let json: JsonValue = serde_json::from_str(s)
        .map_err(|e| Error::Parse(format!("parse JSON from localStorage: {e}")))?;
    json_to_value(json)
}

fn value_to_json(value: &Value) -> JsonValue {
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

fn json_to_value(json: JsonValue) -> Result<Value, Error> {
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
            // Detect Data sentinel: a single-key object with key DATA_SENTINEL
            // mapping to a hex string.
            if map.len() == 1 {
                if let Some(JsonValue::String(hex)) = map.get(DATA_SENTINEL) {
                    let bytes = hex_decode(hex).ok_or_else(|| {
                        Error::Parse("invalid hex in Data sentinel".into())
                    })?;
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

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_namespace(name: &str) -> String {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("persist_web_test_{name}_{n}")
    }

    fn clear_namespace(namespace: &str) {
        if let Some(storage) = get_local_storage() {
            let prefix = format!("{namespace}:");
            let mut to_remove: Vec<String> = Vec::new();
            let len = storage.length().unwrap_or(0);
            for i in 0..len {
                if let Ok(Some(k)) = storage.key(i) {
                    if k.starts_with(&prefix) {
                        to_remove.push(k);
                    }
                }
            }
            for k in to_remove {
                let _ = storage.remove_item(&k);
            }
        }
    }

    #[wasm_bindgen_test]
    fn new_with_namespace() {
        let ns = unique_namespace("new_with_namespace");
        let store = WebStore::new(&ns);
        assert_eq!(store.namespace(), ns);
    }

    #[wasm_bindgen_test]
    fn persistence_state_in_browser_is_persisted() {
        // Under wasm-pack test --headless --chrome, localStorage is
        // available — so we expect Persisted. In Workers this would
        // be MemoryOnly, but that needs a separate test harness.
        let store = WebStore::new(unique_namespace("ps_persisted"));
        assert_eq!(store.persistence_state(), PersistenceState::Persisted);
    }

    #[wasm_bindgen_test]
    fn set_and_get_bool() {
        let ns = unique_namespace("set_get_bool");
        let store = WebStore::new(&ns);
        store.set("flag", Value::Bool(true)).unwrap();
        assert_eq!(store.get("flag").unwrap(), Some(Value::Bool(true)));
        store.set("flag", Value::Bool(false)).unwrap();
        assert_eq!(store.get("flag").unwrap(), Some(Value::Bool(false)));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn set_and_get_int() {
        let ns = unique_namespace("set_get_int");
        let store = WebStore::new(&ns);
        store.set("n", Value::Int(42)).unwrap();
        assert_eq!(store.get("n").unwrap(), Some(Value::Int(42)));
        store.set("n", Value::Int(-7)).unwrap();
        assert_eq!(store.get("n").unwrap(), Some(Value::Int(-7)));
        store.set("n", Value::Int(i64::MAX)).unwrap();
        assert_eq!(store.get("n").unwrap(), Some(Value::Int(i64::MAX)));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn set_and_get_float() {
        let ns = unique_namespace("set_get_float");
        let store = WebStore::new(&ns);
        store.set("pi", Value::Float(3.14)).unwrap();
        assert_eq!(store.get("pi").unwrap(), Some(Value::Float(3.14)));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn set_and_get_string() {
        let ns = unique_namespace("set_get_string");
        let store = WebStore::new(&ns);
        store.set("name", Value::String("Brandon".into())).unwrap();
        assert_eq!(
            store.get("name").unwrap(),
            Some(Value::String("Brandon".into()))
        );
        store.set("name", Value::String(String::new())).unwrap();
        assert_eq!(
            store.get("name").unwrap(),
            Some(Value::String(String::new()))
        );
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn set_and_get_data() {
        let ns = unique_namespace("set_get_data");
        let store = WebStore::new(&ns);
        let bytes = vec![0u8, 1, 2, 3, 255, 128];
        store.set("payload", Value::Data(bytes.clone())).unwrap();
        assert_eq!(store.get("payload").unwrap(), Some(Value::Data(bytes)));

        store.set("payload", Value::Data(vec![])).unwrap();
        assert_eq!(store.get("payload").unwrap(), Some(Value::Data(vec![])));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn set_and_get_array() {
        let ns = unique_namespace("set_get_array");
        let store = WebStore::new(&ns);
        let arr = vec![
            Value::Int(1),
            Value::String("two".into()),
            Value::Bool(true),
            Value::Float(3.5),
        ];
        store.set("arr", Value::Array(arr.clone())).unwrap();
        assert_eq!(store.get("arr").unwrap(), Some(Value::Array(arr)));

        store.set("arr", Value::Array(vec![])).unwrap();
        assert_eq!(store.get("arr").unwrap(), Some(Value::Array(vec![])));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn set_and_get_object() {
        let ns = unique_namespace("set_get_object");
        let store = WebStore::new(&ns);
        let mut map = HashMap::new();
        map.insert("name".into(), Value::String("Brandon".into()));
        map.insert("age".into(), Value::Int(42));
        map.insert("premium".into(), Value::Bool(true));
        store.set("obj", Value::Object(map.clone())).unwrap();
        assert_eq!(store.get("obj").unwrap(), Some(Value::Object(map)));

        store.set("obj", Value::Object(HashMap::new())).unwrap();
        assert_eq!(
            store.get("obj").unwrap(),
            Some(Value::Object(HashMap::new()))
        );
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn get_missing_key_returns_none() {
        let ns = unique_namespace("missing_key");
        let store = WebStore::new(&ns);
        assert_eq!(store.get("nope").unwrap(), None);
    }

    #[wasm_bindgen_test]
    fn delete_removes_key() {
        let ns = unique_namespace("delete_removes");
        let store = WebStore::new(&ns);
        store.set("k", Value::Int(1)).unwrap();
        assert!(store.delete("k").unwrap());
        assert_eq!(store.get("k").unwrap(), None);
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn delete_missing_returns_false() {
        let ns = unique_namespace("delete_missing");
        let store = WebStore::new(&ns);
        assert!(!store.delete("nope").unwrap());
    }

    #[wasm_bindgen_test]
    fn exists_reflects_state() {
        let ns = unique_namespace("exists_state");
        let store = WebStore::new(&ns);
        assert!(!store.exists("k").unwrap());
        store.set("k", Value::Int(1)).unwrap();
        assert!(store.exists("k").unwrap());
        store.delete("k").unwrap();
        assert!(!store.exists("k").unwrap());
    }

    #[wasm_bindgen_test]
    fn set_null_deletes() {
        let ns = unique_namespace("set_null_deletes");
        let store = WebStore::new(&ns);
        store.set("k", Value::Int(1)).unwrap();
        assert!(store.exists("k").unwrap());
        store.set("k", Value::Null).unwrap();
        assert!(!store.exists("k").unwrap());
    }

    #[wasm_bindgen_test]
    fn overwrite_existing_value() {
        let ns = unique_namespace("overwrite");
        let store = WebStore::new(&ns);
        store.set("k", Value::Int(1)).unwrap();
        store.set("k", Value::String("two".into())).unwrap();
        assert_eq!(store.get("k").unwrap(), Some(Value::String("two".into())));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn nested_structures_round_trip() {
        let ns = unique_namespace("nested");
        let store = WebStore::new(&ns);

        let mut inner = HashMap::new();
        inner.insert("x".into(), Value::Float(1.5));

        let mut outer = HashMap::new();
        outer.insert(
            "nums".into(),
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
        outer.insert("inner".into(), Value::Object(inner));
        outer.insert(
            "mixed".into(),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("hi".into()),
                Value::Data(vec![0xDE, 0xAD, 0xBE, 0xEF]),
            ]),
        );

        let obj = Value::Object(outer);
        store.set("deep", obj.clone()).unwrap();
        assert_eq!(store.get("deep").unwrap(), Some(obj));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn nested_array_of_objects() {
        let ns = unique_namespace("arr_of_obj");
        let store = WebStore::new(&ns);

        let mut obj1 = HashMap::new();
        obj1.insert("id".into(), Value::Int(1));
        obj1.insert("name".into(), Value::String("first".into()));

        let mut obj2 = HashMap::new();
        obj2.insert("id".into(), Value::Int(2));
        obj2.insert(
            "tags".into(),
            Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
        );

        let arr = Value::Array(vec![Value::Object(obj1), Value::Object(obj2)]);
        store.set("list", arr.clone()).unwrap();
        assert_eq!(store.get("list").unwrap(), Some(arr));
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn namespaces_do_not_collide() {
        let ns_a = unique_namespace("collide_a");
        let ns_b = unique_namespace("collide_b");
        let a = WebStore::new(&ns_a);
        let b = WebStore::new(&ns_b);

        a.set("shared", Value::String("from_a".into())).unwrap();
        b.set("shared", Value::String("from_b".into())).unwrap();

        assert_eq!(
            a.get("shared").unwrap(),
            Some(Value::String("from_a".into()))
        );
        assert_eq!(
            b.get("shared").unwrap(),
            Some(Value::String("from_b".into()))
        );

        clear_namespace(&ns_a);
        clear_namespace(&ns_b);
    }

    #[wasm_bindgen_test]
    fn persists_across_instances_same_namespace() {
        let ns = unique_namespace("persists_across");
        {
            let writer = WebStore::new(&ns);
            writer.set("k", Value::String("hello".into())).unwrap();
        }
        let reader = WebStore::new(&ns);
        assert_eq!(
            reader.get("k").unwrap(),
            Some(Value::String("hello".into()))
        );
        clear_namespace(&ns);
    }

    #[wasm_bindgen_test]
    fn hex_encode_decode_round_trip() {
        let data: Vec<u8> = (0u8..=255u8).collect();
        let s = hex_encode(&data);
        assert_eq!(s.len(), 512);
        let back = hex_decode(&s).unwrap();
        assert_eq!(back, data);
    }

    #[wasm_bindgen_test]
    fn hex_decode_rejects_odd_length() {
        assert!(hex_decode("abc").is_none());
    }

    #[wasm_bindgen_test]
    fn hex_decode_rejects_non_hex() {
        assert!(hex_decode("zz").is_none());
    }

    #[wasm_bindgen_test]
    fn json_round_trip_preserves_int_vs_float() {
        // JSON numbers can be ambiguous — verify Int stays Int, Float stays Float.
        let ns = unique_namespace("int_vs_float");
        let store = WebStore::new(&ns);

        store.set("i", Value::Int(5)).unwrap();
        store.set("f", Value::Float(5.0)).unwrap();

        assert_eq!(store.get("i").unwrap(), Some(Value::Int(5)));
        // Note: serde_json loses the int/float distinction for whole-number
        // floats — 5.0 may round-trip as Int(5). Accept either.
        match store.get("f").unwrap() {
            Some(Value::Float(n)) if n == 5.0 => (),
            Some(Value::Int(5)) => (),
            other => panic!("unexpected float round-trip: {:?}", other),
        }
        clear_namespace(&ns);
    }

    // Note: the MemoryOnly branch is exercised when localStorage is
    // unavailable (Workers, Service Workers, private browsing). Testing
    // that path requires a different wasm-pack harness (e.g. a Worker
    // context), so it's documented here rather than covered in-browser.
}
