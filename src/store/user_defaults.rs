#![cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos"
))]

use std::collections::HashMap;

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSNull, NSNumber, NSString, NSUserDefaults};

use crate::error::Error;
use crate::store::Store;
use crate::value::Value;

/// NSUserDefaults-backed store for Apple platforms.
///
/// Uses `standardUserDefaults` when constructed via [`UserDefaultsStore::standard`],
/// or a suite (app group) when constructed via [`UserDefaultsStore::with_suite`].
pub struct UserDefaultsStore {
    suite_name: Option<String>,
}

impl UserDefaultsStore {
    /// Use the standard NSUserDefaults (the current process's defaults).
    pub fn standard() -> Self {
        Self { suite_name: None }
    }

    /// Use NSUserDefaults for the given suite name (e.g. an app group identifier).
    pub fn with_suite(suite_name: impl Into<String>) -> Self {
        Self {
            suite_name: Some(suite_name.into()),
        }
    }

    fn defaults(&self) -> Retained<NSUserDefaults> {
        match &self.suite_name {
            None => NSUserDefaults::standardUserDefaults(),
            Some(name) => {
                let ns_name = NSString::from_str(name);
                NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&ns_name))
                    .expect("initWithSuiteName returned nil — suite name is reserved")
            }
        }
    }
}

impl Store for UserDefaultsStore {
    fn get(&self, key: &str) -> Result<Option<Value>, Error> {
        let defaults = self.defaults();
        let ns_key = NSString::from_str(key);
        let obj = defaults.objectForKey(&ns_key);
        match obj {
            None => Ok(None),
            Some(obj) => Ok(Some(any_object_to_value(&obj)?)),
        }
    }

    fn set(&self, key: &str, value: Value) -> Result<(), Error> {
        let defaults = self.defaults();
        let ns_key = NSString::from_str(key);
        match value {
            Value::Null => {
                defaults.removeObjectForKey(&ns_key);
            }
            Value::Bool(b) => defaults.setBool_forKey(b, &ns_key),
            Value::Int(n) => defaults.setInteger_forKey(n as isize, &ns_key),
            Value::Float(n) => defaults.setDouble_forKey(n, &ns_key),
            other => {
                let obj = value_to_ns_object(other)?;
                // SAFETY: obj is a plist-compatible NSObject (NSString, NSData,
                // NSArray of plist objects, or NSDictionary of NSString → plist
                // objects), which is the contract of setObject:forKey:.
                unsafe {
                    defaults.setObject_forKey(Some(&obj), &ns_key);
                }
            }
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<bool, Error> {
        let defaults = self.defaults();
        let ns_key = NSString::from_str(key);
        let existed = defaults.objectForKey(&ns_key).is_some();
        if existed {
            defaults.removeObjectForKey(&ns_key);
        }
        Ok(existed)
    }

    fn exists(&self, key: &str) -> Result<bool, Error> {
        let defaults = self.defaults();
        let ns_key = NSString::from_str(key);
        Ok(defaults.objectForKey(&ns_key).is_some())
    }
}

/// Convert a `Value` into a retained NSObject suitable for
/// `-setObject:forKey:`. Caller must handle top-level `Value::Null`
/// separately (NSUserDefaults uses `-removeObjectForKey:` for nil).
///
/// `Value::Null` nested inside arrays/objects is encoded as NSNull.
/// NSUserDefaults' plist format technically doesn't accept NSNull, but
/// producing NSNull here keeps the in-memory representation honest; if
/// the caller actually sets such a value it will fail at the
/// `setObject:forKey:` boundary via an Objective-C exception.
fn value_to_ns_object(value: Value) -> Result<Retained<AnyObject>, Error> {
    Ok(match value {
        Value::Null => {
            return Err(Error::Custom(
                "Value::Null is not supported inside arrays/objects for NSUserDefaults \
                 (plist format does not allow null). Omit the key instead."
                    .into(),
            ));
        }
        Value::Bool(b) => {
            let num = NSNumber::numberWithBool(b);
            Retained::<NSNumber>::into(num)
        }
        Value::Int(n) => {
            let num = NSNumber::numberWithLongLong(n);
            Retained::<NSNumber>::into(num)
        }
        Value::Float(n) => {
            let num = NSNumber::numberWithDouble(n);
            Retained::<NSNumber>::into(num)
        }
        Value::String(s) => {
            let ns = NSString::from_str(&s);
            Retained::<NSString>::into(ns)
        }
        Value::Data(bytes) => {
            let data = NSData::with_bytes(&bytes);
            Retained::<NSData>::into(data)
        }
        Value::Array(items) => {
            let mut objects: Vec<Retained<AnyObject>> = Vec::with_capacity(items.len());
            for item in items {
                objects.push(value_to_ns_object(item)?);
            }
            let refs: Vec<&AnyObject> = objects.iter().map(|o| &**o).collect();
            let arr = NSArray::<AnyObject>::from_slice(&refs);
            Retained::<NSArray<AnyObject>>::into(arr)
        }
        Value::Object(map) => {
            let mut keys: Vec<Retained<NSString>> = Vec::with_capacity(map.len());
            let mut values: Vec<Retained<AnyObject>> = Vec::with_capacity(map.len());
            for (k, v) in map {
                keys.push(NSString::from_str(&k));
                values.push(value_to_ns_object(v)?);
            }
            let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
            let val_refs: Vec<&AnyObject> = values.iter().map(|v| &**v).collect();
            let dict = NSDictionary::<NSString, AnyObject>::from_slices(&key_refs, &val_refs);
            Retained::<NSDictionary<NSString, AnyObject>>::into(dict)
        }
    })
}

/// Convert an `AnyObject` retrieved from NSUserDefaults into a `Value`.
///
/// NSUserDefaults stores plist types: NSNumber (bool/int/float),
/// NSString, NSData, NSArray, NSDictionary. We inspect the dynamic class
/// to figure out which variant to produce. NSNumber is ambiguous — bool
/// NSNumbers are singletons of `__NSCFBoolean`, so we compare their class
/// against a freshly-constructed bool NSNumber to detect them.
fn any_object_to_value(obj: &AnyObject) -> Result<Value, Error> {
    if obj.downcast_ref::<NSNull>().is_some() {
        return Ok(Value::Null);
    }
    if let Some(s) = obj.downcast_ref::<NSString>() {
        return Ok(Value::String(ns_string_to_rust(s)));
    }
    if let Some(d) = obj.downcast_ref::<NSData>() {
        return Ok(Value::Data(d.to_vec()));
    }
    if let Some(num) = obj.downcast_ref::<NSNumber>() {
        return Ok(ns_number_to_value(num));
    }
    if let Some(arr) = obj.downcast_ref::<NSArray>() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr.iter() {
            out.push(any_object_to_value(&item)?);
        }
        return Ok(Value::Array(out));
    }
    if let Some(dict) = obj.downcast_ref::<NSDictionary>() {
        let (keys, values) = dict.to_vecs();
        let mut out = HashMap::with_capacity(keys.len());
        for (key_obj, val_obj) in keys.into_iter().zip(values.into_iter()) {
            let key_str = key_obj
                .downcast_ref::<NSString>()
                .ok_or_else(|| Error::Parse("NSDictionary key is not an NSString".into()))?;
            let key = ns_string_to_rust(key_str);
            out.insert(key, any_object_to_value(&val_obj)?);
        }
        return Ok(Value::Object(out));
    }
    Err(Error::Parse(format!(
        "unsupported NSUserDefaults value type: {}",
        obj.class().name().to_string_lossy()
    )))
}

fn ns_string_to_rust(s: &NSString) -> String {
    autoreleasepool(|pool| {
        // SAFETY: we immediately copy the &str into an owned String and
        // do not move it outside the autorelease pool.
        unsafe { s.to_str(pool) }.to_owned()
    })
}

fn ns_number_to_value(num: &NSNumber) -> Value {
    if is_boolean(num) {
        Value::Bool(num.boolValue())
    } else {
        // objCType is a single-char encoding (possibly multi-char for
        // structs, but NSNumber only stores primitives). 'f' and 'd' are
        // float and double; everything else is integral.
        let ptr = num.objCType();
        // SAFETY: objCType returns a non-null pointer to a static,
        // null-terminated C string describing the Objective-C type encoding.
        let first = unsafe { *ptr.as_ptr() } as u8;
        match first {
            b'f' | b'd' => Value::Float(num.doubleValue()),
            _ => Value::Int(num.longLongValue()),
        }
    }
}

/// Detect whether an NSNumber is a boolean.
///
/// On Apple platforms, `@YES`/`@NO`/`+numberWithBool:` all return instances
/// of the private `__NSCFBoolean` class rather than a generic NSNumber.
/// Comparing the dynamic class to that of a freshly-created bool NSNumber
/// reliably distinguishes booleans from integers — both share objCType
/// `"c"` (char), so objCType alone is not sufficient.
fn is_boolean(num: &NSNumber) -> bool {
    let reference = NSNumber::numberWithBool(true);
    std::ptr::eq(num.class() as *const _, reference.class() as *const _)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Generate a unique key prefix for test isolation. Each test gets its
    /// own prefix so tests running in parallel don't collide.
    fn unique_prefix(name: &str) -> String {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(
            "persist_test_{}_{}_{}_{}",
            name,
            std::process::id(),
            n,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    }

    /// Clean up all keys written under a prefix.
    fn cleanup(store: &UserDefaultsStore, prefix: &str, keys: &[&str]) {
        for k in keys {
            let _ = store.delete(&format!("{prefix}_{k}"));
        }
    }

    #[test]
    fn set_and_get_bool() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_and_get_bool");
        let key = format!("{prefix}_flag");

        store.set(&key, Value::Bool(true)).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Bool(true)));

        store.set(&key, Value::Bool(false)).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Bool(false)));

        store.delete(&key).unwrap();
    }

    #[test]
    fn set_and_get_int() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_and_get_int");
        let key = format!("{prefix}_n");

        store.set(&key, Value::Int(42)).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Int(42)));

        store.set(&key, Value::Int(-7)).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Int(-7)));

        // Large int
        store.set(&key, Value::Int(i64::MAX)).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Int(i64::MAX)));

        store.delete(&key).unwrap();
    }

    #[test]
    fn set_and_get_float() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_and_get_float");
        let key = format!("{prefix}_pi");

        store.set(&key, Value::Float(2.5)).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Float(2.5)));

        store.delete(&key).unwrap();
    }

    #[test]
    fn set_and_get_string() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_and_get_string");
        let key = format!("{prefix}_name");

        store.set(&key, Value::String("Brandon".into())).unwrap();
        assert_eq!(
            store.get(&key).unwrap(),
            Some(Value::String("Brandon".into()))
        );

        // Empty string
        store.set(&key, Value::String(String::new())).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::String(String::new())));

        store.delete(&key).unwrap();
    }

    #[test]
    fn set_and_get_data() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_and_get_data");
        let key = format!("{prefix}_payload");

        let bytes = vec![0u8, 1, 2, 3, 255, 128];
        store.set(&key, Value::Data(bytes.clone())).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Data(bytes)));

        // Empty data
        store.set(&key, Value::Data(vec![])).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Data(vec![])));

        store.delete(&key).unwrap();
    }

    #[test]
    fn set_and_get_array() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_and_get_array");
        let key = format!("{prefix}_arr");

        let arr = vec![
            Value::Int(1),
            Value::String("two".into()),
            Value::Bool(true),
            Value::Float(3.5),
        ];
        store.set(&key, Value::Array(arr.clone())).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Array(arr)));

        // Empty array
        store.set(&key, Value::Array(vec![])).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Array(vec![])));

        store.delete(&key).unwrap();
    }

    #[test]
    fn set_and_get_object() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_and_get_object");
        let key = format!("{prefix}_obj");

        let mut map = HashMap::new();
        map.insert("name".into(), Value::String("Brandon".into()));
        map.insert("age".into(), Value::Int(42));
        map.insert("premium".into(), Value::Bool(true));

        store.set(&key, Value::Object(map.clone())).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::Object(map)));

        // Empty object
        store.set(&key, Value::Object(HashMap::new())).unwrap();
        assert_eq!(
            store.get(&key).unwrap(),
            Some(Value::Object(HashMap::new()))
        );

        store.delete(&key).unwrap();
    }

    #[test]
    fn get_missing_key_returns_none() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("get_missing");
        let key = format!("{prefix}_nope");

        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn delete_removes_key() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("delete_removes");
        let key = format!("{prefix}_k");

        store.set(&key, Value::Int(1)).unwrap();
        assert!(store.delete(&key).unwrap());
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn delete_missing_returns_false() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("delete_missing");
        let key = format!("{prefix}_nope");

        assert!(!store.delete(&key).unwrap());
    }

    #[test]
    fn exists_reflects_state() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("exists_state");
        let key = format!("{prefix}_k");

        assert!(!store.exists(&key).unwrap());
        store.set(&key, Value::Int(1)).unwrap();
        assert!(store.exists(&key).unwrap());
        store.delete(&key).unwrap();
        assert!(!store.exists(&key).unwrap());
    }

    #[test]
    fn set_null_deletes() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("set_null_deletes");
        let key = format!("{prefix}_k");

        store.set(&key, Value::Int(1)).unwrap();
        assert!(store.exists(&key).unwrap());
        store.set(&key, Value::Null).unwrap();
        assert!(!store.exists(&key).unwrap());
    }

    #[test]
    fn overwrite_existing_value() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("overwrite");
        let key = format!("{prefix}_k");

        store.set(&key, Value::Int(1)).unwrap();
        store.set(&key, Value::String("two".into())).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(Value::String("two".into())));

        store.delete(&key).unwrap();
    }

    #[test]
    fn nested_array_of_objects() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("nested_arr_obj");
        let key = format!("{prefix}_k");

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
        store.set(&key, arr.clone()).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(arr));

        store.delete(&key).unwrap();
    }

    #[test]
    fn nested_object_with_arrays() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("nested_obj_arr");
        let key = format!("{prefix}_k");

        let mut map = HashMap::new();
        map.insert(
            "nums".into(),
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
        let mut inner = HashMap::new();
        inner.insert("x".into(), Value::Float(1.5));
        map.insert("inner".into(), Value::Object(inner));
        map.insert(
            "mixed".into(),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("hi".into()),
                Value::Int(42),
            ]),
        );

        let obj = Value::Object(map);
        store.set(&key, obj.clone()).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(obj));

        store.delete(&key).unwrap();
    }

    #[test]
    fn persists_across_instances() {
        let prefix = unique_prefix("persists_across");
        let key = format!("{prefix}_k");

        let writer = UserDefaultsStore::standard();
        writer.set(&key, Value::String("hello".into())).unwrap();
        drop(writer);

        let reader = UserDefaultsStore::standard();
        assert_eq!(
            reader.get(&key).unwrap(),
            Some(Value::String("hello".into()))
        );

        reader.delete(&key).unwrap();
    }

    #[test]
    fn bool_and_int_are_distinct() {
        // Regression: bool must round-trip as Bool, not Int(0)/Int(1).
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("bool_int_distinct");
        let bkey = format!("{prefix}_b");
        let ikey = format!("{prefix}_i");

        store.set(&bkey, Value::Bool(true)).unwrap();
        store.set(&ikey, Value::Int(1)).unwrap();

        assert_eq!(store.get(&bkey).unwrap(), Some(Value::Bool(true)));
        assert_eq!(store.get(&ikey).unwrap(), Some(Value::Int(1)));

        cleanup(&store, &prefix, &["b", "i"]);
    }

    #[test]
    fn null_inside_array_errors() {
        // NSUserDefaults' plist format does not support NSNull, so Null
        // inside an Array/Object errors at set time rather than silently
        // corrupting data.
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("null_in_array_errors");
        let key = format!("{prefix}_k");

        let arr = Value::Array(vec![Value::Int(1), Value::Null]);
        assert!(store.set(&key, arr).is_err());
    }

    #[test]
    fn null_inside_object_errors() {
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("null_in_object_errors");
        let key = format!("{prefix}_k");

        let mut map = HashMap::new();
        map.insert("a".into(), Value::Null);
        assert!(store.set(&key, Value::Object(map)).is_err());
    }

    #[test]
    fn int_and_float_are_distinct() {
        // Regression: setInteger: and setDouble: should round-trip distinctly.
        let store = UserDefaultsStore::standard();
        let prefix = unique_prefix("int_float_distinct");
        let ikey = format!("{prefix}_i");
        let fkey = format!("{prefix}_f");

        store.set(&ikey, Value::Int(5)).unwrap();
        store.set(&fkey, Value::Float(5.0)).unwrap();

        assert_eq!(store.get(&ikey).unwrap(), Some(Value::Int(5)));
        assert_eq!(store.get(&fkey).unwrap(), Some(Value::Float(5.0)));

        cleanup(&store, &prefix, &["i", "f"]);
    }
}
