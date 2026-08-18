use super::OpIterator;
use crate::Managers;
use common::physical::TupleAssignments;
use common::prelude::*;
use common::table::IndexInfo;
use common::traits::state_tracker_trait::StateTrackerTrait;
use common::traits::storage_trait::StorageTrait;
use common::traits::transaction_manager_trait::TransactionManagerTrait;

/// Update operator
pub struct Update {
    schema: TableSchema,
    open: bool,
    managers: &'static Managers,
    _container_id: ContainerId,
    tid: TransactionId,
    assignments: TupleAssignments,
    /// Every index defined on this table, pre-resolved by the planner
    /// (raw, 0-based column offsets per index) so maintenance doesn't need
    /// its own catalog access.
    indexes: Vec<IndexInfo>,
    child: Box<dyn OpIterator>,
    count: usize,
    /// The child's rows, drained fully in `open()` before any updates are
    /// issued. Necessary because the scan below holds a read latch on
    /// whatever page it's currently positioned on across `next()` calls
    /// (see `HeapFileIter`); updating a row on that same page while the
    /// scan still holds it would fail to acquire the write latch.
    pending: std::vec::IntoIter<Tuple>,
}

impl Update {
    pub fn new(
        managers: &'static Managers,
        container_id: &ContainerId,
        tid: TransactionId,
        assignments: TupleAssignments,
        indexes: Vec<IndexInfo>,
        child: Box<dyn OpIterator>,
    ) -> Self {
        Self {
            schema: child.get_schema().clone(),
            open: false,
            managers,
            _container_id: *container_id,
            tid,
            assignments,
            indexes,
            child,
            count: 0,
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

impl OpIterator for Update {
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
        let next = self.pending.next();
        if let Some(mut tuple) = next {
            let id = match tuple.value_id {
                Some(id) => id,
                None => {
                    return Err(CrustyError::CrustyError(
                        "No value id set for record. Cannot update".to_string(),
                    ));
                }
            };
            // Field values before the assignments are applied, needed to
            // compute each index's *old* key so its stale entry can be removed.
            let old_tuple = tuple.clone();

            // Update values
            self.managers
                .tm
                .pre_update_record(&mut tuple, &id, &self.tid, &self.assignments)?;
            for (field_idx, new_value) in &self.assignments {
                tuple.set_field(*field_idx, new_value.clone());
            }
            // Persist change
            let res = self
                .managers
                .sm
                .update_value(tuple.to_bytes(), id, self.tid);
            //Check result
            match res {
                Ok(new_value_id) => {
                    // notify txn manager
                    self.managers.tm.post_update_record(
                        &mut tuple,
                        &new_value_id,
                        &id,
                        &self.tid,
                        &self.assignments,
                    )?;

                    // Maintain every index on this table: remove the stale
                    // (old_key, old_rid) entry and add the current one. This
                    // is unconditional (not just for indexed columns that
                    // changed) because `new_value_id` can differ from `id`
                    // when the record moved to a new page/slot, which every
                    // index's stored RID needs to reflect regardless of
                    // whether its own key columns changed.
                    for index in &self.indexes {
                        let old_key = Self::extract_key(&old_tuple, &index.columns);
                        let new_key = Self::extract_key(&tuple, &index.columns);
                        self.managers.im.delete_entry(index.index_id, &old_key, id)?;
                        self.managers
                            .im
                            .insert_entry(index.index_id, new_key, new_value_id)?;
                    }

                    self.count += 1;

                    // Update state tracker
                    self.managers
                        .stats
                        .set_ts(new_value_id.container_id, self.tid.id());
                }
                Err(e) => {
                    return Err(e);
                }
            }
            return Ok(Some(tuple));
        }
        Ok(next)
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

#[cfg(test)]
#[allow(unused_must_use)]
mod test {
    //use super::*;
    //use common::ids::TransactionId;
}
