//! Minimal JNI test bridge for persist's SharedPreferencesStore.
//!
//! Two JNI exports:
//! - `nativeInit(context)` — calls `init_android`, consumes the env.
//! - `nativeRunTests()` — exercises every Value type through the full
//!   JNI → SharedPreferences path. Returns "" on success, error on failure.

use std::collections::HashMap;

use jni::objects::{JClass, JObject};
use jni::sys::jstring;
use jni::JNIEnv;

use persist::{init_android, SharedPreferencesStore, Store, Value};

#[no_mangle]
pub unsafe extern "system" fn Java_la_sovran_persist_test_PersistTestBridge_nativeInit(
    env: JNIEnv,
    _class: JClass,
    context: JObject,
) {
    init_android(env, context);
}

#[no_mangle]
pub unsafe extern "system" fn Java_la_sovran_persist_test_PersistTestBridge_nativeRunTests(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let msg = match run_tests() {
        Ok(()) => String::new(),
        Err(e) => e,
    };
    env.new_string(&msg)
        .expect("failed to create result string")
        .into_raw()
}

fn run_tests() -> Result<(), String> {
    let store = SharedPreferencesStore::new("persist_ci_test");

    // --- Bool ---
    store.set("bool_t", Value::Bool(true)).map_err(e)?;
    eq(
        store.get("bool_t").map_err(e)?,
        Some(Value::Bool(true)),
        "bool true",
    )?;
    store.set("bool_f", Value::Bool(false)).map_err(e)?;
    eq(
        store.get("bool_f").map_err(e)?,
        Some(Value::Bool(false)),
        "bool false",
    )?;

    // --- Int ---
    store.set("int", Value::Int(42)).map_err(e)?;
    eq(store.get("int").map_err(e)?, Some(Value::Int(42)), "int 42")?;
    store.set("int_max", Value::Int(i64::MAX)).map_err(e)?;
    eq(
        store.get("int_max").map_err(e)?,
        Some(Value::Int(i64::MAX)),
        "int max",
    )?;
    store.set("int_min", Value::Int(i64::MIN)).map_err(e)?;
    eq(
        store.get("int_min").map_err(e)?,
        Some(Value::Int(i64::MIN)),
        "int min",
    )?;

    // --- Float (lossless via to_bits/from_bits) ---
    store
        .set("float_pi", Value::Float(std::f64::consts::PI))
        .map_err(e)?;
    eq(
        store.get("float_pi").map_err(e)?,
        Some(Value::Float(std::f64::consts::PI)),
        "float pi exact",
    )?;
    store.set("float_max", Value::Float(f64::MAX)).map_err(e)?;
    eq(
        store.get("float_max").map_err(e)?,
        Some(Value::Float(f64::MAX)),
        "float max",
    )?;

    // --- String ---
    store.set("str", Value::String("hello".into())).map_err(e)?;
    eq(
        store.get("str").map_err(e)?,
        Some(Value::String("hello".into())),
        "string",
    )?;
    store
        .set("str_empty", Value::String(String::new()))
        .map_err(e)?;
    eq(
        store.get("str_empty").map_err(e)?,
        Some(Value::String(String::new())),
        "empty string",
    )?;

    // --- Data ---
    let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
    store.set("data", Value::Data(bytes.clone())).map_err(e)?;
    eq(
        store.get("data").map_err(e)?,
        Some(Value::Data(bytes)),
        "data",
    )?;

    // --- Array ---
    let arr = vec![
        Value::Int(1),
        Value::String("two".into()),
        Value::Bool(true),
    ];
    store.set("arr", Value::Array(arr.clone())).map_err(e)?;
    eq(
        store.get("arr").map_err(e)?,
        Some(Value::Array(arr)),
        "array",
    )?;

    // --- Object ---
    let mut map = HashMap::new();
    map.insert("name".into(), Value::String("persist".into()));
    map.insert("version".into(), Value::Int(1));
    store.set("obj", Value::Object(map.clone())).map_err(e)?;
    eq(
        store.get("obj").map_err(e)?,
        Some(Value::Object(map)),
        "object",
    )?;

    // --- Null deletes ---
    store.set("del_null", Value::Int(1)).map_err(e)?;
    check(
        store.exists("del_null").map_err(e)?,
        true,
        "exists before null",
    )?;
    store.set("del_null", Value::Null).map_err(e)?;
    check(
        store.exists("del_null").map_err(e)?,
        false,
        "gone after null",
    )?;

    // --- Delete ---
    store.set("del", Value::Int(1)).map_err(e)?;
    check(store.delete("del").map_err(e)?, true, "delete returns true")?;
    eq(store.get("del").map_err(e)?, None, "get after delete")?;
    check(
        store.delete("del").map_err(e)?,
        false,
        "delete missing returns false",
    )?;

    // --- Missing key ---
    eq(store.get("nonexistent").map_err(e)?, None, "missing key")?;

    // --- Overwrite changes type ---
    store.set("morph", Value::Int(1)).map_err(e)?;
    store
        .set("morph", Value::String("now string".into()))
        .map_err(e)?;
    eq(
        store.get("morph").map_err(e)?,
        Some(Value::String("now string".into())),
        "overwrite changes type",
    )?;

    // --- Nested ---
    let mut inner = HashMap::new();
    inner.insert("x".into(), Value::Float(1.5));
    inner.insert("bytes".into(), Value::Data(vec![0xFF, 0x00]));
    let nested = Value::Array(vec![
        Value::Object(inner),
        Value::String("hi".into()),
        Value::Bool(true),
    ]);
    store.set("nested", nested.clone()).map_err(e)?;
    eq(store.get("nested").map_err(e)?, Some(nested), "nested")?;

    // Clean up.
    for key in [
        "bool_t",
        "bool_f",
        "int",
        "int_max",
        "int_min",
        "float_pi",
        "float_max",
        "str",
        "str_empty",
        "data",
        "arr",
        "obj",
        "morph",
        "nested",
    ] {
        let _ = store.delete(key);
    }

    Ok(())
}

fn e(err: persist::Error) -> String {
    format!("{err}")
}

fn eq(got: Option<Value>, expected: Option<Value>, label: &str) -> Result<(), String> {
    if got != expected {
        Err(format!("{label}: expected {expected:?}, got {got:?}"))
    } else {
        Ok(())
    }
}

fn check<T: PartialEq + std::fmt::Debug>(got: T, expected: T, label: &str) -> Result<(), String> {
    if got != expected {
        Err(format!("{label}: expected {expected:?}, got {got:?}"))
    } else {
        Ok(())
    }
}
