use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::error::Error;
use crate::store::Store;
use crate::value::Value;

/// Serialization format for file-backed stores.
pub trait Format: Send + Sync {
    /// Serialize a map of key-value pairs to a string.
    fn serialize(data: &HashMap<String, Value>) -> Result<String, Error>;
    /// Deserialize a string into a map of key-value pairs.
    fn deserialize(text: &str) -> Result<HashMap<String, Value>, Error>;
}

/// File-backed persistence store, generic over serialization format.
///
/// Supports cached mode (default) where data is held in memory and flushed
/// on write, and uncached mode where every operation hits disk.
///
/// Writes are atomic: data is written to a temp file then renamed over the
/// target, so the file is never half-written.
pub struct FileBackedStore<F: Format> {
    path: PathBuf,
    cached: bool,
    cache: Mutex<Option<HashMap<String, Value>>>,
    _format: std::marker::PhantomData<F>,
}

impl<F: Format> FileBackedStore<F> {
    /// Create a new file-backed store at the given path.
    /// Defaults to cached mode.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cached: true,
            cache: Mutex::new(None),
            _format: std::marker::PhantomData,
        }
    }

    /// Set whether to cache data in memory (default: true).
    pub fn cached(mut self, cached: bool) -> Self {
        self.cached = cached;
        self
    }

    /// Load data from disk. Returns empty map if file doesn't exist.
    fn load_from_disk(&self) -> Result<HashMap<String, Value>, Error> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let text = fs::read_to_string(&self.path)?;
        if text.trim().is_empty() {
            return Ok(HashMap::new());
        }
        F::deserialize(&text)
    }

    /// Write data to disk atomically (write temp + rename).
    fn write_to_disk(&self, data: &HashMap<String, Value>) -> Result<(), Error> {
        let text = F::serialize(data)?;

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        // Atomic write: temp file + rename
        let tmp_path = self.tmp_path();
        fs::write(&tmp_path, text)?;
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    /// Get the temp file path for atomic writes.
    fn tmp_path(&self) -> PathBuf {
        let mut tmp = self.path.clone();
        let name = tmp
            .file_name()
            .map(|n| format!(".{}.tmp", n.to_string_lossy()))
            .unwrap_or_else(|| ".persist.tmp".into());
        tmp.set_file_name(name);
        tmp
    }

    /// Get the current data, loading from disk if needed.
    fn read_data(&self) -> Result<HashMap<String, Value>, Error> {
        if self.cached {
            let mut cache = self.cache.lock().unwrap();
            if cache.is_none() {
                *cache = Some(self.load_from_disk()?);
            }
            Ok(cache.as_ref().unwrap().clone())
        } else {
            self.load_from_disk()
        }
    }

    /// Mutate data and write to disk.
    fn mutate_data(
        &self,
        f: impl FnOnce(&mut HashMap<String, Value>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.cached {
            let mut cache = self.cache.lock().unwrap();
            if cache.is_none() {
                *cache = Some(self.load_from_disk()?);
            }
            let data = cache.as_mut().unwrap();
            f(data)?;
            self.write_to_disk(data)?;
        } else {
            let mut data = self.load_from_disk()?;
            f(&mut data)?;
            self.write_to_disk(&data)?;
        }
        Ok(())
    }
}

impl<F: Format> Store for FileBackedStore<F> {
    fn get(&self, key: &str) -> Result<Option<Value>, Error> {
        let data = self.read_data()?;
        Ok(data.get(key).cloned())
    }

    fn set(&self, key: &str, value: Value) -> Result<(), Error> {
        // Setting Null is equivalent to delete (especially for TOML)
        if matches!(value, Value::Null) {
            self.delete(key)?;
            return Ok(());
        }
        self.mutate_data(|data| {
            data.insert(key.to_owned(), value);
            Ok(())
        })
    }

    fn delete(&self, key: &str) -> Result<bool, Error> {
        let mut existed = false;
        self.mutate_data(|data| {
            existed = data.remove(key).is_some();
            Ok(())
        })?;
        Ok(existed)
    }

    fn exists(&self, key: &str) -> Result<bool, Error> {
        let data = self.read_data()?;
        Ok(data.contains_key(key))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;

    /// Trivial format for testing file_backed logic without depending on
    /// json.rs or toml.rs. Each line is "key=value" with value as debug repr.
    /// Not used outside tests.
    pub(crate) struct TestFormat;

    impl Format for TestFormat {
        fn serialize(data: &HashMap<String, Value>) -> Result<String, Error> {
            let mut lines: Vec<String> = data.iter().map(|(k, v)| format!("{k}={v}")).collect();
            lines.sort(); // deterministic output
            Ok(lines.join("\n"))
        }

        fn deserialize(text: &str) -> Result<HashMap<String, Value>, Error> {
            let mut map = HashMap::new();
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let (key, val) = line
                    .split_once('=')
                    .ok_or_else(|| Error::Parse(format!("invalid line: {line}")))?;
                // Simple: treat everything as a string for testing
                map.insert(key.to_owned(), Value::String(val.to_owned()));
            }
            Ok(map)
        }
    }

    type TestStore = FileBackedStore<TestFormat>;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("persist_tests");
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        // Also clean up tmp file
        let mut tmp = path.to_path_buf();
        let name = tmp
            .file_name()
            .map(|n: &std::ffi::OsStr| format!(".{}.tmp", n.to_string_lossy()))
            .unwrap_or_default();
        tmp.set_file_name(name);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn creates_file_on_first_write() {
        let path = temp_path("creates_file.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        assert!(!path.exists());
        store.set("key", Value::String("value".into())).unwrap();
        assert!(path.exists());

        cleanup(&path);
    }

    #[test]
    fn get_missing_key_returns_none() {
        let path = temp_path("get_missing.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        assert_eq!(store.get("nope").unwrap(), None);

        cleanup(&path);
    }

    #[test]
    fn set_then_get() {
        let path = temp_path("set_get.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        store.set("name", Value::String("Brandon".into())).unwrap();
        assert_eq!(
            store.get("name").unwrap(),
            Some(Value::String("Brandon".into()))
        );

        cleanup(&path);
    }

    #[test]
    fn persists_across_instances() {
        let path = temp_path("persist_across.txt");
        cleanup(&path);

        {
            let store = TestStore::new(&path);
            store.set("key", Value::String("value".into())).unwrap();
        }
        {
            let store = TestStore::new(&path);
            assert_eq!(
                store.get("key").unwrap(),
                Some(Value::String("value".into()))
            );
        }

        cleanup(&path);
    }

    #[test]
    fn delete_existing_key() {
        let path = temp_path("delete_existing.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        store.set("key", Value::String("value".into())).unwrap();
        assert!(store.delete("key").unwrap());
        assert_eq!(store.get("key").unwrap(), None);

        cleanup(&path);
    }

    #[test]
    fn delete_missing_key() {
        let path = temp_path("delete_missing.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        assert!(!store.delete("nope").unwrap());

        cleanup(&path);
    }

    #[test]
    fn exists_reflects_state() {
        let path = temp_path("exists.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        assert!(!store.exists("key").unwrap());
        store.set("key", Value::String("value".into())).unwrap();
        assert!(store.exists("key").unwrap());
        store.delete("key").unwrap();
        assert!(!store.exists("key").unwrap());

        cleanup(&path);
    }

    #[test]
    fn set_overwrites() {
        let path = temp_path("overwrite.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        store.set("key", Value::String("first".into())).unwrap();
        store.set("key", Value::String("second".into())).unwrap();
        assert_eq!(
            store.get("key").unwrap(),
            Some(Value::String("second".into()))
        );

        cleanup(&path);
    }

    #[test]
    fn set_null_deletes_key() {
        let path = temp_path("null_deletes.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        store.set("key", Value::String("value".into())).unwrap();
        store.set("key", Value::Null).unwrap();
        assert_eq!(store.get("key").unwrap(), None);

        cleanup(&path);
    }

    #[test]
    fn handles_empty_file() {
        let path = temp_path("empty_file.txt");
        cleanup(&path);
        fs::write(&path, "").unwrap();

        let store = TestStore::new(&path);
        assert_eq!(store.get("key").unwrap(), None);

        cleanup(&path);
    }

    #[test]
    fn handles_nonexistent_file() {
        let path = temp_path("nonexistent.txt");
        cleanup(&path);

        let store = TestStore::new(&path);
        assert_eq!(store.get("key").unwrap(), None);

        cleanup(&path);
    }

    #[test]
    fn creates_parent_directories() {
        let path = temp_path("nested/dir/settings.txt");
        cleanup(&path);
        let _ = fs::remove_dir_all(path.parent().unwrap());

        let store = TestStore::new(&path);
        store.set("key", Value::String("value".into())).unwrap();
        assert!(path.exists());

        cleanup(&path);
        let _ = fs::remove_dir_all(temp_path("nested"));
    }

    #[test]
    fn uncached_reads_from_disk_each_time() {
        let path = temp_path("uncached.txt");
        cleanup(&path);

        let store = TestStore::new(&path).cached(false);
        store.set("key", Value::String("value".into())).unwrap();

        // Write directly to disk behind the store's back
        fs::write(&path, "key=modified").unwrap();

        // Uncached store should see the external change
        assert_eq!(
            store.get("key").unwrap(),
            Some(Value::String("modified".into()))
        );

        cleanup(&path);
    }

    #[test]
    fn cached_does_not_see_external_changes() {
        let path = temp_path("cached_stale.txt");
        cleanup(&path);

        let store = TestStore::new(&path).cached(true);
        store.set("key", Value::String("value".into())).unwrap();

        // Write directly to disk behind the store's back
        fs::write(&path, "key=modified").unwrap();

        // Cached store should still see the old value
        assert_eq!(
            store.get("key").unwrap(),
            Some(Value::String("value".into()))
        );

        cleanup(&path);
    }

    #[test]
    fn corrupt_file_returns_error() {
        let path = temp_path("corrupt.txt");
        cleanup(&path);
        fs::write(&path, "not_valid_data_no_equals_sign").unwrap();

        let store = TestStore::new(&path);
        assert!(store.get("key").is_err());

        cleanup(&path);
    }
}
