mod convert;
mod error;
#[cfg(feature = "serde")]
mod serde_compat;
mod store;
mod value;

pub use error::Error;
#[cfg(target_os = "android")]
pub use store::shared_preferences::init_android;
#[cfg(feature = "json")]
pub use store::JsonFileStore;
#[cfg(target_os = "android")]
pub use store::SharedPreferencesStore;
#[cfg(feature = "toml")]
pub use store::TomlFileStore;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos"
))]
pub use store::UserDefaultsStore;
pub use store::{FileBackedStore, Format, Store};
#[cfg(target_arch = "wasm32")]
pub use store::{PersistenceState, WebStore};
pub use value::{CoerceFrom, Value};

#[cfg(any(feature = "json", feature = "toml"))]
use std::path::PathBuf;

/// Cross-platform data persistence with pluggable backends.
pub struct Persist {
    store: Box<dyn Store>,
}

impl Persist {
    /// Create a Persist instance with any Store implementation.
    pub fn new(store: impl Store + 'static) -> Self {
        Self {
            store: Box::new(store),
        }
    }

    /// Convenience: JSON file-backed store (cached by default).
    #[cfg(feature = "json")]
    pub fn json(path: impl Into<PathBuf>) -> Self {
        Self::new(JsonFileStore::new(path))
    }

    /// Convenience: TOML file-backed store (cached by default).
    #[cfg(feature = "toml")]
    pub fn toml(path: impl Into<PathBuf>) -> Self {
        Self::new(TomlFileStore::new(path))
    }

    /// Convenience: standard NSUserDefaults-backed store (Apple platforms).
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos"
    ))]
    pub fn user_defaults() -> Self {
        Self::new(UserDefaultsStore::standard())
    }

    /// Convenience: NSUserDefaults-backed store for a given suite/app-group.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos"
    ))]
    pub fn user_defaults_with_suite(suite_name: impl Into<String>) -> Self {
        Self::new(UserDefaultsStore::with_suite(suite_name))
    }

    /// Convenience: browser-backed store (localStorage with in-memory fallback).
    #[cfg(target_arch = "wasm32")]
    pub fn web(namespace: impl Into<String>) -> Self {
        Self::new(WebStore::new(namespace))
    }

    /// Convenience: Android `SharedPreferences`-backed store. Requires
    /// [`init_android`] to have been called by the host app.
    #[cfg(target_os = "android")]
    pub fn shared_preferences(prefs_name: impl Into<String>) -> Self {
        Self::new(SharedPreferencesStore::new(prefs_name))
    }

    /// Set a value. Accepts anything that converts Into<Value>.
    pub fn set(&self, key: &str, value: impl Into<Value>) -> Result<(), Error> {
        self.store.set(key, value.into())
    }

    /// Get a raw Value.
    pub fn get(&self, key: &str) -> Result<Option<Value>, Error> {
        self.store.get(key)
    }

    /// Get a typed value. Returns error on type mismatch.
    pub fn get_as<T: TryFrom<Value, Error = Error>>(&self, key: &str) -> Result<Option<T>, Error> {
        match self.store.get(key)? {
            Some(v) => Ok(Some(T::try_from(v)?)),
            None => Ok(None),
        }
    }

    /// Get a typed value with best-effort coercion.
    pub fn get_coerce<T: CoerceFrom<Value>>(&self, key: &str) -> Result<Option<T>, Error> {
        match self.store.get(key)? {
            Some(v) => Ok(Some(T::coerce_from(v)?)),
            None => Ok(None),
        }
    }

    /// Delete a key. Returns true if it existed.
    pub fn delete(&self, key: &str) -> Result<bool, Error> {
        self.store.delete(key)
    }

    /// Check if a key exists.
    pub fn exists(&self, key: &str) -> Result<bool, Error> {
        self.store.exists(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory store for testing.
    struct MemoryStore {
        data: Mutex<HashMap<String, Value>>,
    }

    impl MemoryStore {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Store for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<Value>, Error> {
            Ok(self.data.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: Value) -> Result<(), Error> {
            self.data.lock().unwrap().insert(key.to_owned(), value);
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<bool, Error> {
            Ok(self.data.lock().unwrap().remove(key).is_some())
        }

        fn exists(&self, key: &str) -> Result<bool, Error> {
            Ok(self.data.lock().unwrap().contains_key(key))
        }
    }

    fn test_persist() -> Persist {
        Persist::new(MemoryStore::new())
    }

    // --- Basic set/get ---

    #[test]
    fn set_and_get_string() {
        let p = test_persist();
        p.set("name", "Brandon").unwrap();
        let val = p.get("name").unwrap();
        assert_eq!(val, Some(Value::String("Brandon".into())));
    }

    #[test]
    fn set_and_get_int() {
        let p = test_persist();
        p.set("age", 42i64).unwrap();
        let val = p.get("age").unwrap();
        assert_eq!(val, Some(Value::Int(42)));
    }

    #[test]
    fn set_and_get_bool() {
        let p = test_persist();
        p.set("active", true).unwrap();
        let val = p.get("active").unwrap();
        assert_eq!(val, Some(Value::Bool(true)));
    }

    #[test]
    fn set_and_get_float() {
        let p = test_persist();
        p.set("pi", 3.25f64).unwrap();
        let val = p.get("pi").unwrap();
        assert_eq!(val, Some(Value::Float(3.25)));
    }

    #[test]
    fn set_and_get_data() {
        let p = test_persist();
        p.set("payload", vec![0u8, 1, 2, 3]).unwrap();
        let val = p.get("payload").unwrap();
        assert_eq!(val, Some(Value::Data(vec![0, 1, 2, 3])));
    }

    #[test]
    fn get_missing_key_returns_none() {
        let p = test_persist();
        assert_eq!(p.get("nope").unwrap(), None);
    }

    #[test]
    fn set_overwrites_existing() {
        let p = test_persist();
        p.set("key", "first").unwrap();
        p.set("key", "second").unwrap();
        assert_eq!(p.get("key").unwrap(), Some(Value::String("second".into())));
    }

    // --- Delete ---

    #[test]
    fn delete_existing_key() {
        let p = test_persist();
        p.set("key", "value").unwrap();
        assert!(p.delete("key").unwrap());
        assert_eq!(p.get("key").unwrap(), None);
    }

    #[test]
    fn delete_missing_key() {
        let p = test_persist();
        assert!(!p.delete("nope").unwrap());
    }

    // --- Exists ---

    #[test]
    fn exists_reflects_state() {
        let p = test_persist();
        assert!(!p.exists("key").unwrap());
        p.set("key", "value").unwrap();
        assert!(p.exists("key").unwrap());
        p.delete("key").unwrap();
        assert!(!p.exists("key").unwrap());
    }

    // --- get_as (strict typed) ---

    #[test]
    fn get_as_string() {
        let p = test_persist();
        p.set("name", "Brandon").unwrap();
        let val: String = p.get_as("name").unwrap().unwrap();
        assert_eq!(val, "Brandon");
    }

    #[test]
    fn get_as_i64() {
        let p = test_persist();
        p.set("age", 42i64).unwrap();
        let val: i64 = p.get_as("age").unwrap().unwrap();
        assert_eq!(val, 42);
    }

    #[test]
    fn get_as_type_mismatch() {
        let p = test_persist();
        p.set("age", 42i64).unwrap();
        let result = p.get_as::<String>("age");
        assert!(result.is_err());
    }

    #[test]
    fn get_as_missing_key() {
        let p = test_persist();
        let val = p.get_as::<String>("nope").unwrap();
        assert_eq!(val, None);
    }

    // --- get_coerce ---

    #[test]
    fn get_coerce_int_to_string() {
        let p = test_persist();
        p.set("port", 8080i64).unwrap();
        let val: String = p.get_coerce("port").unwrap().unwrap();
        assert_eq!(val, "8080");
    }

    #[test]
    fn get_coerce_string_to_int() {
        let p = test_persist();
        p.set("port", "8080").unwrap();
        let val: i64 = p.get_coerce("port").unwrap().unwrap();
        assert_eq!(val, 8080);
    }

    #[test]
    fn get_coerce_int_to_bool() {
        let p = test_persist();
        p.set("flag", 1i64).unwrap();
        let val: bool = p.get_coerce("flag").unwrap().unwrap();
        assert!(val);
    }

    #[test]
    fn get_coerce_bool_to_int() {
        let p = test_persist();
        p.set("flag", true).unwrap();
        let val: i64 = p.get_coerce("flag").unwrap().unwrap();
        assert_eq!(val, 1);
    }

    #[test]
    fn get_coerce_float_to_int_truncates() {
        let p = test_persist();
        p.set("val", 3.9f64).unwrap();
        let val: i64 = p.get_coerce("val").unwrap().unwrap();
        assert_eq!(val, 3);
    }

    #[test]
    fn get_coerce_nonsensical_fails() {
        let p = test_persist();
        p.set("arr", Value::Array(vec![Value::Int(1)])).unwrap();
        let result = p.get_coerce::<i64>("arr");
        assert!(result.is_err());
    }

    #[test]
    fn get_coerce_missing_key() {
        let p = test_persist();
        let val = p.get_coerce::<String>("nope").unwrap();
        assert_eq!(val, None);
    }

    // --- Convenience constructors ---

    #[cfg(feature = "json")]
    #[test]
    fn persist_json_convenience() {
        let dir = std::env::temp_dir().join("persist_tests_lib");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("convenience.json");
        let _ = std::fs::remove_file(&path);

        let p = Persist::json(&path);
        p.set("name", "Brandon").unwrap();
        drop(p);

        let p = Persist::json(&path);
        let name: String = p.get_as("name").unwrap().unwrap();
        assert_eq!(name, "Brandon");

        let _ = std::fs::remove_file(&path);
    }

    #[cfg(feature = "toml")]
    #[test]
    fn persist_toml_convenience() {
        let dir = std::env::temp_dir().join("persist_tests_lib");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("convenience.toml");
        let _ = std::fs::remove_file(&path);

        let p = Persist::toml(&path);
        p.set("port", 8080i64).unwrap();
        drop(p);

        let p = Persist::toml(&path);
        let port: i64 = p.get_as("port").unwrap().unwrap();
        assert_eq!(port, 8080);

        let _ = std::fs::remove_file(&path);
    }
}
