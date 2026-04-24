# persist

Cross-platform key-value persistence for Rust. One API, pluggable backends.

```rust
let p = Persist::json("settings.json");

p.set("name", "Brandon")?;
p.set("port", 8080i64)?;
p.set("debug", true)?;

let name: String = p.get_as("name")?.unwrap();
let port: String = p.get_coerce("port")?.unwrap(); // "8080"
```

## Why

Every platform has its own storage API. `UserDefaults` on Apple, `SharedPreferences` on Android, `localStorage` in the browser, flat files everywhere else. This crate gives you a single `Store` trait across all of them with a rich value type that round-trips cleanly.

## Stores

| Store | Target | Backend |
|-------|--------|---------|
| `JsonFileStore` | All | JSON file (feature `json`) |
| `TomlFileStore` | All | TOML file (feature `toml`) |
| `UserDefaultsStore` | macOS, iOS, tvOS, watchOS | `NSUserDefaults` via objc2 |
| `SharedPreferencesStore` | Android | `SharedPreferences` via JNI |
| `WebStore` | WASM | `localStorage` with in-memory fallback |

File-backed stores cache in memory and write atomically (write-tmp, rename).

## Value types

```rust
enum Value {
    Null, Bool(bool), Int(i64), Float(f64),
    String(String), Data(Vec<u8>),
    Array(Vec<Value>), Object(HashMap<String, Value>),
}
```

Binary data (`Data`) round-trips through JSON/TOML via a `$persist:data` hex sentinel. Floats on Android are stored losslessly using `f64::to_bits()` through `putLong`.

## Typed access

`get_as<T>` is strict — type mismatch is an error:

```rust
p.set("port", 8080i64)?;
let port: i64 = p.get_as("port")?.unwrap();     // Ok
let port: String = p.get_as("port")?.unwrap();   // Error
```

`get_coerce<T>` does best-effort conversion:

```rust
let port: String = p.get_coerce("port")?.unwrap(); // "8080"
let flag: bool = p.get_coerce("flag")?.unwrap();    // 1 → true
let n: i64 = p.get_coerce("n")?.unwrap();           // "42" → 42
```

## Features

```toml
[dependencies]
persist = "1"                          # core only (bring your own Store impl)
persist = { version = "1", features = ["json"] }   # + JsonFileStore
persist = { version = "1", features = ["toml"] }   # + TomlFileStore
persist = { version = "1", features = ["serde"] }  # + serde Serialize/Deserialize for Value
```

Platform stores (Apple, Android, WASM) are auto-enabled by target — no feature flags needed.

## Platform setup

### Apple

Works out of the box:

```rust
let p = Persist::user_defaults();
// or with an app group:
let p = Persist::user_defaults_with_suite("group.com.example.app");
```

### Android

The host app must call `init_android` once at startup to provide the JVM and Context:

```rust
// In your JNI_OnLoad or early initialization:
unsafe { persist::init_android(env, context); }

// Then anywhere:
let p = Persist::shared_preferences("my_prefs");
```

### WASM

```rust
let p = Persist::web("my-app");

// Check if localStorage is available or we're in memory-only mode:
match store.persistence_state() {
    PersistenceState::Persisted => { /* localStorage available */ }
    PersistenceState::MemoryOnly => { /* Workers, Service Workers, etc. */ }
}
```

## Custom stores

Implement the `Store` trait:

```rust
impl Store for MyStore {
    fn get(&self, key: &str) -> Result<Option<Value>, Error>;
    fn set(&self, key: &str, value: Value) -> Result<(), Error>;
    fn delete(&self, key: &str) -> Result<bool, Error>;
    fn exists(&self, key: &str) -> Result<bool, Error>;
}

let p = Persist::new(MyStore::new());
```

## License

MIT
