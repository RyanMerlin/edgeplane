pub mod error;
pub mod store;
pub mod types;

pub use error::ReceiptsError;
pub use store::ReceiptStore;
pub use types::{Receipt, ReceiptFilter};

/// Default path: `~/.edgeplane/state/receipts.db` (or `$EP_HOME/state/receipts.db`)
pub fn default_db_path() -> std::path::PathBuf {
    edgeplaned_paths::receipts_db_path()
}
