pub mod backend;
pub mod index;
pub mod memory;
pub mod snapshot;
pub mod store;

#[cfg(feature = "rocksdb")]
pub mod rocks;

#[cfg(feature = "rocksdb")]
pub mod key_prefix;

pub use backend::{StorageBackend, StorageError};
pub use index::{MessageIndex, TipManager};
pub use memory::MemoryBackend;
pub use snapshot::{Snapshot, SnapshotManager};
pub use store::{StorageConfig, StorageEngine};

#[cfg(feature = "rocksdb")]
pub use rocks::RocksBackend;
