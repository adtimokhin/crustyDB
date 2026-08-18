use super::OpIterator;
use crate::Managers;
use common::prelude::*;
use common::table::IndexInfo;
use common::traits::storage_trait::StorageTrait;

/// Delete operator: consumes every tuple its child produces, deletes the
/// underlying storage record for each, and removes the corresponding entry
/// from every index defined on the table. Yields the deleted tuples (so the
/// caller can report a row count).
pub struct Delete {
    schema: TableSchema,
    open: bool,
    managers: &'static Managers,
    tid: TransactionId,
    indexes: Vec<IndexInfo>,
    child: Box<dyn OpIterator>,
    /// The child's rows, drained fully in `open()` before any deletes are
    /// issued. Necessary because the scan below holds a read latch on
    /// whatever page it's currently positioned on across `next()` calls
    /// (see `HeapFileIter`); deleting a row on that same page while the
    /// scan still holds it would fail to acquire the write latch.
    pending: std::vec::IntoIter<Tuple>,
}

impl Delete {
    pub fn new(
        managers: &'static Managers,
        tid: TransactionId,
        indexes: Vec<IndexInfo>,
        child: Box<dyn OpIterator>,
    ) -> Self {
        Self {
            schema: child.get_schema().clone(),
            open: false,
            managers,
            tid,
            indexes,
            child,
            pending: Vec::new().into_iter(),
        }
    }

    fn extract_key(tuple: &Tuple, columns: &[usize]) -> Vec<Field> {
        columns
            .iter()
            .map(|&i| tuple.get_field(i).expect("index column out of range").clone())
            .collect()
    }
}

impl OpIterator for Delete {
    fn configure(&mut self, will_rewind: bool) {
        self.child.configure(will_rewind);
    }

    fn open(&mut self) -> Result<(), CrustyError> {
        if !self.open {
            self.child.open()?;
            let mut rows = Vec::new();
            while let Some(t) = self.child.next()? {
                rows.push(t);
            }
            self.pending = rows.into_iter();
        }
        self.open = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Tuple>, CrustyError> {
        if !self.open {
            panic!("Operator has not been opened")
        }
        if let Some(tuple) = self.pending.next() {
            let id = tuple.value_id.ok_or_else(|| {
                CrustyError::CrustyError("No value id set for record. Cannot delete".to_string())
            })?;

            for index in &self.indexes {
                let key = Self::extract_key(&tuple, &index.columns);
                self.managers.im.delete_entry(index.index_id, &key, id)?;
            }
            self.managers.sm.delete_value(id, self.tid)?;

            Ok(Some(tuple))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), CrustyError> {
        self.child.close()?;
        self.pending = Vec::new().into_iter();
        self.open = false;
        Ok(())
    }

    fn rewind(&mut self) -> Result<(), CrustyError> {
        unimplemented!();
    }

    fn get_schema(&self) -> &TableSchema {
        &self.schema
    }
}
