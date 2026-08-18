use crate::tree::TreeIndex;
use common::error::c_err;
use common::ids::{ColumnId, ContainerId, Permissions, TransactionId, ValueId};
use common::physical::config::ServerConfig;
use common::traits::storage_trait::StorageTrait;
use common::{CrustyError, Field, Tuple};
use dashmap::DashMap;
use heapstore::buffer_pool::buffer_pool::BufferPool;
use log::info;
use std::sync::Arc;

use crate::{StorageManager, TransactionManager};

/// An index's id is also the id of the storage container its B+Tree pages live in.
pub type IndexId = ContainerId;

/// Manages every live B+Tree index in the database: creation (including the
/// initial scan-and-build over existing rows), and per-row maintenance
/// (insert/delete) called from the mutator and the Update/Delete operators.
pub struct IndexManager {
    #[allow(dead_code)]
    config: &'static ServerConfig,
    sm: &'static StorageManager,
    #[allow(dead_code)]
    tm: &'static TransactionManager,
    trees: DashMap<IndexId, Arc<TreeIndex<BufferPool>>>,
}

impl IndexManager {
    pub fn new(
        config: &'static ServerConfig,
        sm: &'static StorageManager,
        tm: &'static TransactionManager,
    ) -> Self {
        Self {
            config,
            sm,
            tm,
            trees: DashMap::new(),
        }
    }

    /// Extract the (possibly composite) index key from a tuple, given the
    /// index's raw (0-based) column offsets.
    fn extract_key(tuple: &Tuple, columns: &[ColumnId]) -> Vec<Field> {
        columns
            .iter()
            .map(|&i| tuple.get_field(i).expect("index column out of range").clone())
            .collect()
    }

    /// Create a brand-new B+Tree index in container `index_id` over `columns`
    /// (raw, 0-based attribute offsets) of table `table_id`, and populate it
    /// by scanning every existing row in the table.
    pub fn create_index(
        &self,
        index_id: IndexId,
        table_id: ContainerId,
        columns: &[ColumnId],
        tid: TransactionId,
    ) -> Result<(), CrustyError> {
        let tree = Arc::new(TreeIndex::new(index_id, self.sm.bp.clone())?);

        let iter = self.sm.get_iterator(table_id, tid, Permissions::ReadOnly);
        for (bytes, value_id) in iter {
            let tuple = Tuple::from_bytes(&bytes);
            let key = Self::extract_key(&tuple, columns);
            tree.insert(key, value_id)?;
        }

        self.trees.insert(index_id, tree);
        Ok(())
    }

    fn get_tree(&self, index_id: IndexId) -> Result<Arc<TreeIndex<BufferPool>>, CrustyError> {
        self.trees
            .get(&index_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| c_err(&format!("Index {} not found", index_id)))
    }

    /// Look up every `ValueId` stored under `key` in the given index.
    pub fn search(&self, index_id: IndexId, key: &[Field]) -> Result<Vec<ValueId>, CrustyError> {
        self.get_tree(index_id)?.search(key)
    }

    /// Add one `(key, rid)` entry to the index. Called from insert/update paths.
    pub fn insert_entry(
        &self,
        index_id: IndexId,
        key: Vec<Field>,
        rid: ValueId,
    ) -> Result<(), CrustyError> {
        self.get_tree(index_id)?.insert(key, rid)
    }

    /// Remove one `(key, rid)` entry from the index. Called from delete/update paths.
    pub fn delete_entry(
        &self,
        index_id: IndexId,
        key: &[Field],
        rid: ValueId,
    ) -> Result<(), CrustyError> {
        self.get_tree(index_id)?.delete(key, rid)
    }

    pub fn shutdown(&self) -> Result<(), CrustyError> {
        // Index pages live in the storage manager's shared buffer pool, which
        // is flushed by `StorageManager::shutdown` (called after this by
        // `Managers::shutdown`). Nothing extra to persist here.
        info!("Index manager shutdown");
        Ok(())
    }

    pub fn reset(&self) -> Result<(), CrustyError> {
        // Drop our handles; the underlying container files are actually
        // removed by `StorageManager::reset` (called after this by
        // `Managers::reset`), which clears the shared buffer pool + catalog.
        self.trees.clear();
        Ok(())
    }
}
