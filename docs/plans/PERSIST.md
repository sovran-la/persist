# Persist

**Date:** 2026-04-23
**Authors:** Brandon Sneed, Porter
**Branch:** TBD

## Goal

A cross-platform data persistence crate with a trait-based interface. Consumers create a store implementation and pass it into `Persist`. The crate ships with built-in file-backed stores (JSON, TOML) behind feature flags. Platform-specific backends (UserDefaults, SharedPreferences, Keychain, etc.) are implemented downstream by platform SDKs.

Small data persistence — settings, preferences, cached config — not a database.

## Design

### Value Type

A `Value` enum covering the JSON type spread plus binary data:

```rust
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
```

### Store Trait

```rust
pub trait Store: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Value>, Error>;
    fn set(&self, key: &str, value: Value) -> Result<(), Error>;
    fn delete(&self, key: &str) -> Result<bool, Error>;
    fn exists(&self, key: &str) -> Result<bool, Error>;
}
```

### Persist Struct

Wraps a `Box<dyn Store>` with convenience constructors and typed access:

```rust
pub struct Persist {
    store: Box<dyn Store>,
}

impl Persist {
    /// Use any custom Store implementation.
    pub fn new(store: impl Store + 'static) -> Self {
        Self { store: Box::new(store) }
    }

    /// Convenience: JSON file-backed store (requires `json` feature).
    pub fn json(path: impl Into<PathBuf>) -> Self {
        Self::new(JsonFileStore::new(path))
    }

    /// Convenience: TOML file-backed store (requires `toml` feature).
    pub fn toml(path: impl Into<PathBuf>) -> Self {
        Self::new(TomlFileStore::new(path))
    }

    /// Set a value. Accepts anything that converts Into<Value>.
    pub fn set(&self, key: &str, value: impl Into<Value>) -> Result<(), Error> { ... }

    /// Get a raw Value.
    pub fn get(&self, key: &str) -> Result<Option<Value>, Error> { ... }

    /// Get a typed value. Returns error on type mismatch.
    pub fn get_as<T: TryFrom<Value>>(&self, key: &str) -> Result<Option<T>, Error> { ... }

    /// Get a typed value with best-effort coercion. Tries harder than get_as.
    pub fn get_coerce<T: CoerceFrom<Value>>(&self, key: &str) -> Result<Option<T>, Error> { ... }

    /// Delete a key. Returns true if it existed.
    pub fn delete(&self, key: &str) -> Result<bool, Error> { ... }

    /// Check if a key exists.
    pub fn exists(&self, key: &str) -> Result<bool, Error> { ... }
}
```

`From<T> for Value` impls for: `&str`, `String`, `bool`, `i64`, `f64`, `Vec<u8>`, `Vec<Value>`, `HashMap<String, Value>`.

`TryFrom<Value> for T` impls for: `String`, `bool`, `i64`, `f64`, `Vec<u8>`, `Vec<Value>`, `HashMap<String, Value>`. Strict — type mismatch is an error.

`CoerceFrom<Value> for T` — best-effort coercion with predictable rules:
- Any scalar → `String` (Display)
- `String` → numeric types (parse, error if unparseable)
- `Int` ↔ `Float` (precision loss possible)
- `Bool` ↔ `Int` (true=1, false=0; reverse: 0=false, 1=true, other = error)
- `Data` ↔ `String` (valid UTF-8 only, otherwise error)
- Nonsensical conversions (`Array` → `Int`, `Object` → `Bool`, etc.) still error

### Caller Ergonomics

```rust
// Setting values — From<T> makes this clean
p.set("name", "Brandon")?;
p.set("age", 42i64)?;
p.set("premium", true)?;
p.set("payload", vec![0u8, 1, 2, 3])?;

// Getting raw Value when you need to inspect/match
let val = p.get("settings")?; // Option<Value>

// Getting typed values when you know what's there
let name: String = p.get_as("name")?.unwrap_or_default();
let age = p.get_as::<i64>("age")?.unwrap_or(0);

// Delete
p.delete("name")?;

// Coercion — stored as Int, read as String
p.set("port", 8080i64)?;
let port: String = p.get_coerce("port")?.unwrap(); // "8080"
// get_as would error here, get_coerce converts
```

`get_as` and `get_coerce` are on `Persist`, not on the `Store` trait — store implementors only deal in `Value`.

### Built-in Stores

**JsonFileStore** (behind `json` feature) — Reads/writes a single JSON file using `serde_json`. Works on all platforms with filesystem access.

**TomlFileStore** (behind `toml` feature) — Reads/writes a single TOML file using the `toml` crate. Works on all platforms with filesystem access.

Both use the shared `FileBackedStore<F>` infrastructure in `store/file_backed.rs` which handles caching, atomic write-rename, and file I/O. The format implementations are thin wrappers that delegate serialization to the mature crates.

### Downstream / Platform Stores (not in this crate)

These are `Store` trait impls that live in platform SDKs or companion crates:

- **UserDefaultsStore** — Apple platforms (macOS, iOS, tvOS, watchOS)
- **SharedPreferencesStore** — Android
- **KeychainStore** — Apple (secure storage)
- **KeystoreStore** — Android (secure storage)
- **LocalStorageStore** — WASM (browser localStorage)
- **RegistryStore** — Windows
- **Custom** — anything implementing `Store`

### Caller Examples

```rust
// Built-in convenience (requires feature flags)
let p = Persist::json("/path/to/settings.json");   // requires `json` feature
let p = Persist::toml("/path/to/settings.toml");   // requires `toml` feature

// Platform-specific (implemented downstream)
let p = Persist::new(UserDefaultsStore::new("com.twilio.sdk"));
let p = Persist::new(SharedPreferencesStore::new(context, "twilio_prefs"));
let p = Persist::new(KeychainStore::new("com.twilio.sdk"));

// Custom
let p = Persist::new(my_custom_store);
```

## Feature Flags

### `json` (optional, off by default)

Enables `JsonFileStore` and `Persist::json()`. Pulls in `serde`, `serde_json`.

### `toml` (optional, off by default)

Enables `TomlFileStore` and `Persist::toml()`. Pulls in `serde`, `toml` crate.

### `serde` (optional, off by default)

Enables `Serialize`/`Deserialize` on `Value` and `From<serde_json::Value>`/`Into<serde_json::Value>` conversions. Automatically enabled by `json` and `toml` features.

### Core (no features)

Zero dependencies. Value, Store trait, Persist, Error, From/TryFrom/CoerceFrom impls. Someone using only a custom `Store` impl pays for nothing they don't use.

## Crate Structure

```
persist/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Persist struct, re-exports
│   ├── value.rs            # Value enum, From/TryFrom/CoerceFrom impls
│   ├── error.rs            # Error type
│   ├── serde_compat.rs     # Serde integration (behind `serde` feature)
│   └── store/
│       ├── mod.rs           # Store trait definition, re-exports
│       ├── file_backed.rs   # Format trait, FileBackedStore<F> (cache, atomic writes)
│       ├── json.rs          # JsonFormat via serde_json (behind `json` feature)
│       ├── toml.rs          # TomlFormat via toml crate (behind `toml` feature)
│       └── user_defaults.rs # UserDefaultsStore (Stage 4)
└── docs/
    └── plans/
        └── PERSIST.md
```

## Implementation Stages

### Stage 1: Create the crate
Cargo.toml, module structure, lib.rs with re-exports. Compiles, does nothing.

**Status: COMPLETE**

### Stage 2: Baseline
Value enum, Store trait, Error type, Persist wrapper, From/TryFrom/CoerceFrom impls. Tests for Value round-tripping and type conversions.

**Status: COMPLETE** — 61 tests passing.

### Stage 3: File stores
JsonFileStore and TomlFileStore with cached/uncached modes, atomic write-rename. Full test suite.

**Status: REWORK NEEDED** — Initial implementation used hand-rolled serializers. Reworking to use serde_json and toml crates behind feature flags.

**Rework scope:**
- Update Cargo.toml: `json` feature → serde + serde_json, `toml` feature → serde + toml crate. Both implicitly enable `serde` feature.
- `store/json.rs`: Delete hand-rolled JSON parser/writer. Implement `Format` trait by converting `HashMap<String, Value>` ↔ `serde_json::Value` and using `serde_json::to_string_pretty` / `serde_json::from_str`. Requires Serialize/Deserialize on Value (from serde_compat.rs). Gate behind `#[cfg(feature = "json")]`.
- `store/toml.rs`: Same approach using `toml` crate. Gate behind `#[cfg(feature = "toml")]`.
- `store/file_backed.rs`: Keep as-is — Format trait, FileBackedStore<F>, caching, atomic writes. No changes needed.
- `serde_compat.rs`: Implement Serialize/Deserialize for Value, plus From/Into conversions with serde_json::Value. Used internally by json/toml stores and available to consumers via `serde` feature.
- `lib.rs`: Gate `Persist::json()` behind `#[cfg(feature = "json")]`, `Persist::toml()` behind `#[cfg(feature = "toml")]`. Re-exports gated accordingly.
- `store/mod.rs`: Gate json/toml module declarations and re-exports behind their feature flags.
- Tests: Keep the same test coverage. Run json tests under `#[cfg(test)]` (features enabled during test). Verify core compiles with no features enabled.

### Stage 3b: Serde compatibility
All serde integration in a single `src/serde_compat.rs` file behind `#[cfg(feature = "serde")]`. Includes: Serialize/Deserialize derives on Value, From<serde_json::Value>/Into<serde_json::Value> conversions, tests. Keeps value.rs clean and zero-dep.

**Note:** Now part of Stage 3 rework since json/toml stores depend on it.

### Stage 4: Platform stores
Platform backends are target-gated (not feature flags). Dependencies are pulled in automatically based on build target.

**Apple (UserDefaultsStore):** Uses `objc2` + `objc2-foundation` for NSUserDefaults. Gated behind `#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos", target_os = "watchos"))]`. Tests run on macOS.

**Android (SharedPreferencesStore):** Uses `jni` crate (target-gated, not feature flagged). Follows sysdirs pattern: `init_android(vm, context)` called by host Kotlin/Java app at startup, stored in `OnceLock`. SharedPreferencesStore uses stored VM/Context for JNI calls. Compound types (Array, Object) JSON-serialized into strings. Data base64-encoded. Conversion logic split into pure Rust (testable locally) and JNI layer (cross-compile checked, integration-tested on device/emulator).

Cargo.toml uses `[target.'cfg(...)'.dependencies]` so consumers never have to enable platform features — the right backend is just there when you build for that target.

**WASM (WebStore):** Uses `web-sys` + `wasm-bindgen` (target-gated). Fallback chain: localStorage (sync, matches Store trait) → memory-only (when localStorage unavailable — Workers, Service Workers, private browsing, certain iframes). `PersistenceState` enum (`Persisted` / `MemoryOnly`) so consumers can check if data is actually durable. Pattern borrowed from TransientDB's WebStore. Fast local feedback loop via `wasm-pack test --headless --chrome`.

**Windows/Linux:** Covered by file stores (JsonFileStore, TomlFileStore). No platform-specific store needed.

**Status: Apple COMPLETE** — 154 tests with --all-features (20 UserDefaults-specific). objc2 v0.6 + objc2-foundation v0.3. Android/WASM/Windows TBD.

### Stage 5: CI
GitHub Actions workflow modeled after sysdirs:
- **Full tests:** Linux, macOS, Windows (native runners)
- **Cross-compile checks:** `cargo check --target aarch64-linux-android`, `cargo check --target wasm32-unknown-unknown`, `cargo check --target aarch64-apple-ios`
- **Lint:** clippy + rustfmt
- **Optional:** Android emulator job for integration tests (nightly or manual, doesn't block PRs)

## Resolved Questions

- **TOML and Null:** Setting a key to `Value::Null` omits the key from the file (equivalent to delete). `get` returns `Ok(None)` for missing keys, so this is seamless.
- **Thread safety on file stores:** Atomic write-rename. Write to temp file in same directory, rename over original. No file locking. Guarantees the file is never half-written. Concurrent write races are the caller's problem.
- **Caching on file stores:** Builder flag on file stores. `cached(true)` (default) loads the file into a `HashMap<String, Value>` on first access, serves reads from memory, flushes on write. `cached(false)` reads from disk every time. Only relevant on file stores — platform backends (UserDefaults, SharedPreferences, etc.) handle their own caching.

```rust
// Default: cached
let store = JsonFileStore::new(path);

// Opt out
let store = JsonFileStore::new(path).cached(false);
```

Convenience constructors (`Persist::json()`, `Persist::toml()`) default to cached.

## Test Plan

### Value
- Round-trip: every `Value` variant → JSON string → parse → equals original
- Round-trip: every `Value` variant → TOML string → parse → equals original
- Nested structures: `Object` containing `Array` containing `Object` etc.
- `Data` variant: binary data round-trips through base64 encoding
- Edge cases: empty strings, empty arrays, empty objects, large integers, float precision

### Store trait / Persist
- `get` on missing key returns `Ok(None)`
- `set` then `get` returns the value
- `set` overwrites existing value
- `delete` existing key returns `Ok(true)`
- `delete` missing key returns `Ok(false)`
- `exists` reflects current state after set/delete

### JsonFileStore
- Creates file on first write
- Persists across drop/re-open (write, drop, re-create from same path, read)
- Handles empty file (fresh state)
- Handles corrupt file (returns error, doesn't panic)
- Concurrent access: atomic write-rename guarantees no partial writes

### TomlFileStore
- Same suite as JsonFileStore
- Null handling: setting `Value::Null` omits key, `get` returns `Ok(None)`
- TOML-specific: nested objects serialize as TOML tables

### Caching
- `cached(true)`: set then get serves from memory without re-reading file
- `cached(true)`: persists across flush — write, read back from new instance, data matches
- `cached(false)`: set writes to disk, get re-reads from disk each time
- `cached(false)`: external modification between writes is picked up on next get

### Feature gating
- Core compiles with no features enabled (`cargo check --no-default-features`)
- `json` feature: JsonFileStore available, Persist::json() works
- `toml` feature: TomlFileStore available, Persist::toml() works
- `serde` feature: Value has Serialize/Deserialize, conversions with serde_json::Value work

### Serde feature
- `Value` round-trips through `serde_json::Value`
- Feature off: code compiles without serde in dependency tree

## Dependencies

Core: none. Optional: `serde` + `serde_json` (via `json` feature), `serde` + `toml` (via `toml` feature).

## Notes

- Name `persist` is currently squatted on crates.io (7 years, no releases). Brandon has reported it. If the name isn't freed, we'll need an alternative.
- This crate is the "Persistence (External Dependency)" referenced in twilio-data-core's RULES_LOADING.md plan. Once available, it slots into the rules loading cache layer.
- sysdirs is a natural companion — file-backed stores can use `sysdirs` for default paths if no explicit path is provided. Whether to make that a dependency or leave path resolution to the caller is TBD.
