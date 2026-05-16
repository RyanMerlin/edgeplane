pub mod error;
pub mod store;
pub mod types;

pub use error::ReceiptsError;
pub use store::ReceiptStore;
pub use types::{Receipt, ReceiptFilter};

/// Default path: `~/.mc/receipts.db` (or `$MC_HOME/receipts.db`)
pub fn default_db_path() -> std::path::PathBuf {
    mc_home_dir().join("receipts.db")
}

fn mc_home_dir() -> std::path::PathBuf {
    if let Ok(val) = std::env::var("MC_HOME") {
        if !val.is_empty() {
            return expand_home(&val);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".mc")
}

fn expand_home(val: &str) -> std::path::PathBuf {
    if let Some(stripped) = val.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    std::path::PathBuf::from(val)
}
