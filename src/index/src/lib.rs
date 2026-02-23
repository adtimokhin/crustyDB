use storage::StorageManager;
/// Re-export Storage manager here for this crate to use. This allows us to change
/// the storage manager by changing one use statement.
use txn_manager::mock_tm::MockTransactionManager as TransactionManager;

// pub use hash::HashIndex;
pub use index_manager::IndexManager;
//pub use tree::TreeIndex;

//mod hash;
mod index_manager;
//mod tree;

//pub mod index_trait;
