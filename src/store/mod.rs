mod file_backed;
#[cfg(feature = "json")]
pub mod json;
#[cfg(feature = "toml")]
pub mod toml;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos"
))]
pub mod user_defaults;
#[cfg(target_arch = "wasm32")]
pub mod web;

use crate::Error;
use crate::Value;

pub use file_backed::{FileBackedStore, Format};
#[cfg(feature = "json")]
pub use json::JsonFileStore;
#[cfg(feature = "toml")]
pub use toml::TomlFileStore;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos"
))]
pub use user_defaults::UserDefaultsStore;
#[cfg(target_arch = "wasm32")]
pub use web::{PersistenceState, WebStore};

/// Trait for persistence backends.
///
/// Implementors deal only in `Value` — typed access (`get_as`, `get_coerce`)
/// lives on `Persist` and is handled automatically.
pub trait Store: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Value>, Error>;
    fn set(&self, key: &str, value: Value) -> Result<(), Error>;
    fn delete(&self, key: &str) -> Result<bool, Error>;
    fn exists(&self, key: &str) -> Result<bool, Error>;
}
