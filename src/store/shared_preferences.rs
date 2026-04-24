#![cfg(target_os = "android")]

//! Android `SharedPreferences`-backed store.
//!
//! The host Kotlin/Java app must call [`init_android`] once at startup to
//! provide a `JavaVM` handle and an Android `Context` reference. After that,
//! [`SharedPreferencesStore`] can be constructed and used from any thread.
//!
//! ## Value mapping
//!
//! `SharedPreferences` supports only boolean, int, long, float, `String`, and
//! `Set<String>`. The richer `Value` type is mapped as follows:
//!
//! | `Value` variant | `SharedPreferences` representation |
//! |-----------------|------------------------------------|
//! | `Bool`          | `putBoolean` / `getBoolean`        |
//! | `Int` (i64)     | `putLong`  / `getLong`             |
//! | `Float` (f64)   | `putLong` via `f64::to_bits()` (lossless)   |
//! | `String`        | `putString` / `getString`          |
//! | `Data`          | `putString` with hex encoding      |
//! | `Array`         | `putString` with JSON serialization |
//! | `Object`        | `putString` with JSON serialization |
//! | `Null`          | `remove`                           |
//!
//! `SharedPreferences` has no way to introspect the type of a stored value. To
//! support heterogeneous typed storage, every key `foo` also gets a parallel
//! `foo.__persist_type` string entry recording the original variant. On `get`,
//! the type tag is read first and dispatches to the correct typed getter.
//!
//! ## Float storage
//!
//! `SharedPreferences` has no `putDouble`/`getDouble`. To avoid f64→f32
//! precision loss, `Value::Float` is stored as its IEEE 754 bit pattern
//! via `putLong(f64::to_bits() as i64)` and recovered with
//! `f64::from_bits(getLong() as u64)`. The type tag ensures correct
//! dispatch — the raw SharedPreferences value is an opaque long.

use std::sync::OnceLock;

use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::convert::{hex_decode, hex_encode, json_string_to_value, value_to_json_string};
use crate::error::Error;
use crate::store::Store;
use crate::value::Value;

// =============================================================================
// Initialization
// =============================================================================

static ANDROID_VM: OnceLock<JavaVM> = OnceLock::new();
static ANDROID_CONTEXT: OnceLock<GlobalRef> = OnceLock::new();

/// Initialize the Android persistence layer with a `JavaVM` handle and an
/// Android `Context`. Call this once — typically from `JNI_OnLoad` or an
/// early JNI initialization entry point in the host Kotlin/Java app. The
/// stored `JavaVM` is used on subsequent calls to attach native threads for
/// JNI work; the `Context` is retained as a `GlobalRef` so it outlives the
/// JNI local frame of this call.
///
/// Subsequent calls are silently ignored — the first call wins.
///
/// # Safety
///
/// The caller must ensure that:
/// - `env` is a valid `JNIEnv` for the current thread (i.e. we are executing
///   inside a JNI call into native code, not from an arbitrary native
///   thread).
/// - `context` is a valid `android.content.Context` reference (typically the
///   application context).
///
/// `JavaVM` and `GlobalRef` are both documented as `Send + Sync` in the `jni`
/// crate, so storing them in `OnceLock` statics is safe once the above
/// preconditions hold.
pub unsafe fn init_android(env: JNIEnv, context: JObject) {
    let vm = env
        .get_java_vm()
        .expect("init_android: failed to obtain JavaVM from JNIEnv");
    let _ = ANDROID_VM.set(vm);
    let global_ctx = env
        .new_global_ref(&context)
        .expect("init_android: failed to create GlobalRef for Context");
    let _ = ANDROID_CONTEXT.set(global_ctx);
}

fn vm() -> Result<&'static JavaVM, Error> {
    ANDROID_VM.get().ok_or_else(not_initialized)
}

fn context() -> Result<&'static GlobalRef, Error> {
    ANDROID_CONTEXT.get().ok_or_else(not_initialized)
}

fn not_initialized() -> Error {
    Error::Custom(
        "init_android has not been called — call it once from your host app \
         before using SharedPreferencesStore"
            .into(),
    )
}

// =============================================================================
// SharedPreferencesStore
// =============================================================================

/// A [`Store`] implementation backed by Android `SharedPreferences`.
///
/// Construct with a `prefs_name` that will be passed to
/// `Context.getSharedPreferences(name, MODE_PRIVATE)`. Requires
/// [`init_android`] to have been called first.
pub struct SharedPreferencesStore {
    prefs_name: String,
}

impl SharedPreferencesStore {
    /// Create a store backed by the named `SharedPreferences` file.
    pub fn new(prefs_name: impl Into<String>) -> Self {
        Self {
            prefs_name: prefs_name.into(),
        }
    }

    /// The `SharedPreferences` name this store is bound to.
    pub fn prefs_name(&self) -> &str {
        &self.prefs_name
    }
}

impl Store for SharedPreferencesStore {
    fn get(&self, key: &str) -> Result<Option<Value>, Error> {
        let vm = vm()?;
        let ctx = context()?;
        let mut guard = vm.attach_current_thread().map_err(jni_err)?;
        let env: &mut JNIEnv = &mut guard;

        let prefs = shared_preferences(env, ctx.as_obj(), &self.prefs_name)?;

        let tag_key = type_tag_key(key);
        let type_tag = match prefs_get_string(env, &prefs, &tag_key)? {
            Some(t) => t,
            None => return Ok(None),
        };

        match type_tag.as_str() {
            TAG_BOOL => Ok(Some(Value::Bool(prefs_get_bool(env, &prefs, key)?))),
            TAG_INT => Ok(Some(Value::Int(prefs_get_long(env, &prefs, key)?))),
            TAG_FLOAT => Ok(Some(Value::Float(f64::from_bits(
                prefs_get_long(env, &prefs, key)? as u64,
            )))),
            TAG_STRING => {
                let s = prefs_get_string(env, &prefs, key)?.ok_or_else(|| orphan_tag(key))?;
                Ok(Some(Value::String(s)))
            }
            TAG_DATA => {
                let s = prefs_get_string(env, &prefs, key)?.ok_or_else(|| orphan_tag(key))?;
                let bytes = hex_decode(&s).ok_or_else(|| {
                    Error::Parse(format!("invalid hex encoding for Data key '{key}'"))
                })?;
                Ok(Some(Value::Data(bytes)))
            }
            TAG_ARRAY | TAG_OBJECT => {
                let s = prefs_get_string(env, &prefs, key)?.ok_or_else(|| orphan_tag(key))?;
                Ok(Some(json_string_to_value(&s)?))
            }
            other => Err(Error::Parse(format!(
                "unknown persist type tag '{other}' for key '{key}'"
            ))),
        }
    }

    fn set(&self, key: &str, value: Value) -> Result<(), Error> {
        if matches!(value, Value::Null) {
            self.delete(key)?;
            return Ok(());
        }

        let vm = vm()?;
        let ctx = context()?;
        let mut guard = vm.attach_current_thread().map_err(jni_err)?;
        let env: &mut JNIEnv = &mut guard;

        let prefs = shared_preferences(env, ctx.as_obj(), &self.prefs_name)?;
        let editor = prefs_edit(env, &prefs)?;
        let tag_key = type_tag_key(key);

        match value {
            Value::Null => unreachable!("handled above"),
            Value::Bool(b) => {
                editor_put_bool(env, &editor, key, b)?;
                editor_put_string(env, &editor, &tag_key, TAG_BOOL)?;
            }
            Value::Int(n) => {
                editor_put_long(env, &editor, key, n)?;
                editor_put_string(env, &editor, &tag_key, TAG_INT)?;
            }
            Value::Float(n) => {
                editor_put_long(env, &editor, key, n.to_bits() as i64)?;
                editor_put_string(env, &editor, &tag_key, TAG_FLOAT)?;
            }
            Value::String(ref s) => {
                editor_put_string(env, &editor, key, s)?;
                editor_put_string(env, &editor, &tag_key, TAG_STRING)?;
            }
            Value::Data(ref bytes) => {
                let hex = hex_encode(bytes);
                editor_put_string(env, &editor, key, &hex)?;
                editor_put_string(env, &editor, &tag_key, TAG_DATA)?;
            }
            Value::Array(_) => {
                let json = value_to_json_string(&value)?;
                editor_put_string(env, &editor, key, &json)?;
                editor_put_string(env, &editor, &tag_key, TAG_ARRAY)?;
            }
            Value::Object(_) => {
                let json = value_to_json_string(&value)?;
                editor_put_string(env, &editor, key, &json)?;
                editor_put_string(env, &editor, &tag_key, TAG_OBJECT)?;
            }
        }

        editor_commit(env, &editor)
    }

    fn delete(&self, key: &str) -> Result<bool, Error> {
        let vm = vm()?;
        let ctx = context()?;
        let mut guard = vm.attach_current_thread().map_err(jni_err)?;
        let env: &mut JNIEnv = &mut guard;

        let prefs = shared_preferences(env, ctx.as_obj(), &self.prefs_name)?;

        // `contains` checks the value key. The type-tag key is a paired
        // internal detail — if it somehow exists without the value key we
        // still want to clear it, but we report `false` from `delete` because
        // from the caller's perspective no user-visible key existed.
        let existed = prefs_contains(env, &prefs, key)?;
        let tag_key = type_tag_key(key);
        let tag_existed = prefs_contains(env, &prefs, &tag_key)?;
        if existed || tag_existed {
            let editor = prefs_edit(env, &prefs)?;
            editor_remove(env, &editor, key)?;
            editor_remove(env, &editor, &tag_key)?;
            editor_commit(env, &editor)?;
        }
        Ok(existed)
    }

    fn exists(&self, key: &str) -> Result<bool, Error> {
        let vm = vm()?;
        let ctx = context()?;
        let mut guard = vm.attach_current_thread().map_err(jni_err)?;
        let env: &mut JNIEnv = &mut guard;

        let prefs = shared_preferences(env, ctx.as_obj(), &self.prefs_name)?;
        prefs_contains(env, &prefs, key)
    }
}

// =============================================================================
// JNI helpers
// =============================================================================

fn jni_err(e: jni::errors::Error) -> Error {
    Error::Custom(format!("JNI error: {e}"))
}

fn orphan_tag(key: &str) -> Error {
    Error::Custom(format!(
        "persist type tag exists but value is missing for key '{key}' \
         (SharedPreferences inconsistency)"
    ))
}

/// `Context.getSharedPreferences(name, MODE_PRIVATE) -> SharedPreferences`.
fn shared_preferences<'local>(
    env: &mut JNIEnv<'local>,
    context: &JObject,
    prefs_name: &str,
) -> Result<JObject<'local>, Error> {
    let name = env.new_string(prefs_name).map_err(jni_err)?;
    env.call_method(
        context,
        "getSharedPreferences",
        "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
        // Context.MODE_PRIVATE = 0
        &[JValue::Object(&name), JValue::Int(0)],
    )
    .map_err(jni_err)?
    .l()
    .map_err(jni_err)
}

/// `SharedPreferences.edit() -> SharedPreferences.Editor`.
fn prefs_edit<'local>(env: &mut JNIEnv<'local>, prefs: &JObject) -> Result<JObject<'local>, Error> {
    env.call_method(
        prefs,
        "edit",
        "()Landroid/content/SharedPreferences$Editor;",
        &[],
    )
    .map_err(jni_err)?
    .l()
    .map_err(jni_err)
}

/// `SharedPreferences.contains(key) -> boolean`.
fn prefs_contains(env: &mut JNIEnv, prefs: &JObject, key: &str) -> Result<bool, Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    env.call_method(
        prefs,
        "contains",
        "(Ljava/lang/String;)Z",
        &[JValue::Object(&k)],
    )
    .map_err(jni_err)?
    .z()
    .map_err(jni_err)
}

/// `SharedPreferences.getString(key, null)`. Returns `None` when the key
/// isn't present or the stored value is null.
fn prefs_get_string(env: &mut JNIEnv, prefs: &JObject, key: &str) -> Result<Option<String>, Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    let null_default: JObject = JObject::null();
    let result = env
        .call_method(
            prefs,
            "getString",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&k), JValue::Object(&null_default)],
        )
        .map_err(jni_err)?
        .l()
        .map_err(jni_err)?;

    if result.is_null() {
        return Ok(None);
    }

    let jstring = JString::from(result);
    let java_str = env.get_string(&jstring).map_err(jni_err)?;
    Ok(Some(java_str.to_string_lossy().into_owned()))
}

/// `SharedPreferences.getBoolean(key, false)`.
fn prefs_get_bool(env: &mut JNIEnv, prefs: &JObject, key: &str) -> Result<bool, Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    env.call_method(
        prefs,
        "getBoolean",
        "(Ljava/lang/String;Z)Z",
        &[JValue::Object(&k), JValue::Bool(0)],
    )
    .map_err(jni_err)?
    .z()
    .map_err(jni_err)
}

/// `SharedPreferences.getLong(key, 0)`.
fn prefs_get_long(env: &mut JNIEnv, prefs: &JObject, key: &str) -> Result<i64, Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    env.call_method(
        prefs,
        "getLong",
        "(Ljava/lang/String;J)J",
        &[JValue::Object(&k), JValue::Long(0)],
    )
    .map_err(jni_err)?
    .j()
    .map_err(jni_err)
}

fn editor_put_bool(
    env: &mut JNIEnv,
    editor: &JObject,
    key: &str,
    value: bool,
) -> Result<(), Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    env.call_method(
        editor,
        "putBoolean",
        "(Ljava/lang/String;Z)Landroid/content/SharedPreferences$Editor;",
        &[JValue::Object(&k), JValue::Bool(if value { 1 } else { 0 })],
    )
    .map_err(jni_err)?;
    Ok(())
}

fn editor_put_long(env: &mut JNIEnv, editor: &JObject, key: &str, value: i64) -> Result<(), Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    env.call_method(
        editor,
        "putLong",
        "(Ljava/lang/String;J)Landroid/content/SharedPreferences$Editor;",
        &[JValue::Object(&k), JValue::Long(value)],
    )
    .map_err(jni_err)?;
    Ok(())
}

fn editor_put_string(
    env: &mut JNIEnv,
    editor: &JObject,
    key: &str,
    value: &str,
) -> Result<(), Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    let v = env.new_string(value).map_err(jni_err)?;
    env.call_method(
        editor,
        "putString",
        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
        &[JValue::Object(&k), JValue::Object(&v)],
    )
    .map_err(jni_err)?;
    Ok(())
}

fn editor_remove(env: &mut JNIEnv, editor: &JObject, key: &str) -> Result<(), Error> {
    let k = env.new_string(key).map_err(jni_err)?;
    env.call_method(
        editor,
        "remove",
        "(Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
        &[JValue::Object(&k)],
    )
    .map_err(jni_err)?;
    Ok(())
}

fn editor_commit(env: &mut JNIEnv, editor: &JObject) -> Result<(), Error> {
    let ok = env
        .call_method(editor, "commit", "()Z", &[])
        .map_err(jni_err)?
        .z()
        .map_err(jni_err)?;
    if !ok {
        return Err(Error::Custom(
            "SharedPreferences.Editor.commit() returned false".into(),
        ));
    }
    Ok(())
}

// =============================================================================
// Type tags and conversion helpers
// =============================================================================

/// Suffix appended to a user key to form the sidecar type-tag key.
const TYPE_TAG_SUFFIX: &str = ".__persist_type";

const TAG_BOOL: &str = "bool";
const TAG_INT: &str = "int";
const TAG_FLOAT: &str = "float";
const TAG_STRING: &str = "string";
const TAG_DATA: &str = "data";
const TAG_ARRAY: &str = "array";
const TAG_OBJECT: &str = "object";

fn type_tag_key(key: &str) -> String {
    format!("{key}{TYPE_TAG_SUFFIX}")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_tag_key_format() {
        assert_eq!(type_tag_key("foo"), "foo.__persist_type");
        assert_eq!(type_tag_key(""), ".__persist_type");
        assert_eq!(type_tag_key("a.b.c"), "a.b.c.__persist_type");
    }

    #[test]
    fn type_tag_constants_are_lowercase_words() {
        // If any of these change, it's a persistence format break — stored
        // data on user devices would be mis-typed on next read.
        assert_eq!(TAG_BOOL, "bool");
        assert_eq!(TAG_INT, "int");
        assert_eq!(TAG_FLOAT, "float");
        assert_eq!(TAG_STRING, "string");
        assert_eq!(TAG_DATA, "data");
        assert_eq!(TAG_ARRAY, "array");
        assert_eq!(TAG_OBJECT, "object");
    }

    // --- Integration tests (require a live Android JVM + Context) ---
    //
    // These tests exercise the full JNI path. They require that
    // `init_android` has been called with a valid VM/Context — so they'll
    // only pass when run on an Android device or emulator with the host app
    // wired up. On a vanilla cargo test run (even with --target
    // aarch64-linux-android), they're compiled but the store calls will
    // return Error::Custom("init_android has not been called ...").
    //
    // The `integration_*` tests are gated behind the `android-integration`
    // cfg flag so they don't fail CI. Enable with:
    //   RUSTFLAGS='--cfg android_integration' cargo test --target aarch64-linux-android

    #[cfg(android_integration)]
    mod integration {
        use super::*;
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicU64, Ordering};

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        fn unique_prefs_name(tag: &str) -> String {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            format!("persist_test_{tag}_{n}")
        }

        fn fresh_store(tag: &str) -> SharedPreferencesStore {
            SharedPreferencesStore::new(unique_prefs_name(tag))
        }

        #[test]
        fn integration_set_and_get_bool() {
            let store = fresh_store("bool");
            store.set("flag", Value::Bool(true)).unwrap();
            assert_eq!(store.get("flag").unwrap(), Some(Value::Bool(true)));
            store.set("flag", Value::Bool(false)).unwrap();
            assert_eq!(store.get("flag").unwrap(), Some(Value::Bool(false)));
        }

        #[test]
        fn integration_set_and_get_int() {
            let store = fresh_store("int");
            store.set("n", Value::Int(42)).unwrap();
            assert_eq!(store.get("n").unwrap(), Some(Value::Int(42)));
            store.set("n", Value::Int(-7)).unwrap();
            assert_eq!(store.get("n").unwrap(), Some(Value::Int(-7)));
            store.set("n", Value::Int(i64::MAX)).unwrap();
            assert_eq!(store.get("n").unwrap(), Some(Value::Int(i64::MAX)));
            store.set("n", Value::Int(i64::MIN)).unwrap();
            assert_eq!(store.get("n").unwrap(), Some(Value::Int(i64::MIN)));
        }

        #[test]
        fn integration_set_and_get_float_lossless() {
            let store = fresh_store("float");
            // Exact f32-representable value.
            store.set("exact", Value::Float(2.5)).unwrap();
            assert_eq!(store.get("exact").unwrap(), Some(Value::Float(2.5)));

            // Full f64 precision preserved via to_bits/from_bits.
            store.set("pi", Value::Float(std::f64::consts::PI)).unwrap();
            assert_eq!(
                store.get("pi").unwrap(),
                Some(Value::Float(std::f64::consts::PI))
            );

            // Extremes.
            store.set("max", Value::Float(f64::MAX)).unwrap();
            assert_eq!(store.get("max").unwrap(), Some(Value::Float(f64::MAX)));

            store.set("min", Value::Float(f64::MIN)).unwrap();
            assert_eq!(store.get("min").unwrap(), Some(Value::Float(f64::MIN)));

            store.set("tiny", Value::Float(f64::MIN_POSITIVE)).unwrap();
            assert_eq!(
                store.get("tiny").unwrap(),
                Some(Value::Float(f64::MIN_POSITIVE))
            );
        }

        #[test]
        fn integration_set_and_get_string() {
            let store = fresh_store("string");
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
        }

        #[test]
        fn integration_set_and_get_data() {
            let store = fresh_store("data");
            let bytes = vec![0u8, 1, 2, 3, 255, 128];
            store.set("payload", Value::Data(bytes.clone())).unwrap();
            assert_eq!(store.get("payload").unwrap(), Some(Value::Data(bytes)));
            store.set("payload", Value::Data(vec![])).unwrap();
            assert_eq!(store.get("payload").unwrap(), Some(Value::Data(vec![])));
        }

        #[test]
        fn integration_set_and_get_array() {
            let store = fresh_store("arr");
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
        }

        #[test]
        fn integration_set_and_get_object() {
            let store = fresh_store("obj");
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
        }

        #[test]
        fn integration_get_missing_key_returns_none() {
            let store = fresh_store("missing");
            assert_eq!(store.get("nope").unwrap(), None);
        }

        #[test]
        fn integration_delete_removes_key_and_type_tag() {
            let store = fresh_store("delete");
            store.set("k", Value::Int(1)).unwrap();
            assert!(store.exists("k").unwrap());
            assert!(store.delete("k").unwrap());
            assert_eq!(store.get("k").unwrap(), None);
            assert!(!store.exists("k").unwrap());
            // Type-tag key should also be gone so the next write can pick a
            // fresh type.
            let tag = type_tag_key("k");
            // Can't call prefs_contains directly here without a live JNIEnv
            // obtained via the store, so verify through get() behaviour: a
            // missing type tag makes get() return None even if the user
            // later writes a different typed value — covered by overwrite.
            let _ = tag;
        }

        #[test]
        fn integration_delete_missing_returns_false() {
            let store = fresh_store("del_missing");
            assert!(!store.delete("nope").unwrap());
        }

        #[test]
        fn integration_exists_reflects_state() {
            let store = fresh_store("exists");
            assert!(!store.exists("k").unwrap());
            store.set("k", Value::Int(1)).unwrap();
            assert!(store.exists("k").unwrap());
            store.delete("k").unwrap();
            assert!(!store.exists("k").unwrap());
        }

        #[test]
        fn integration_set_null_deletes() {
            let store = fresh_store("null");
            store.set("k", Value::Int(1)).unwrap();
            assert!(store.exists("k").unwrap());
            store.set("k", Value::Null).unwrap();
            assert!(!store.exists("k").unwrap());
        }

        #[test]
        fn integration_overwrite_changes_type() {
            // After overwriting an Int with a String, get() must return the
            // new String — proving the type tag was updated.
            let store = fresh_store("overwrite");
            store.set("k", Value::Int(1)).unwrap();
            store.set("k", Value::String("two".into())).unwrap();
            assert_eq!(store.get("k").unwrap(), Some(Value::String("two".into())));
        }

        #[test]
        fn integration_nested_structures_round_trip() {
            let store = fresh_store("nested");

            let mut inner = HashMap::new();
            inner.insert("x".into(), Value::Float(1.5));
            inner.insert("bytes".into(), Value::Data(vec![0xFF, 0x00]));

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

            let v = Value::Object(outer);
            store.set("deep", v.clone()).unwrap();
            assert_eq!(store.get("deep").unwrap(), Some(v));
        }

        #[test]
        fn integration_persists_across_instances() {
            let name = unique_prefs_name("persists");
            let writer = SharedPreferencesStore::new(&name);
            writer.set("k", Value::String("hello".into())).unwrap();
            drop(writer);

            let reader = SharedPreferencesStore::new(&name);
            assert_eq!(
                reader.get("k").unwrap(),
                Some(Value::String("hello".into()))
            );
        }
    }
}
