use super::OpIterator;
use crate::Managers;
use common::ids::Permissions;
use common::prelude::*;
use common::traits::storage_trait::StorageTrait;

/// Index scan operator: a point lookup via a B+Tree index, in place of a full
/// `SeqScan`. Used by the planner instead of `Scan`+`Select` when every
/// column of some index on the table has an equality predicate in the query
/// (see `LogicalRelExpr::IndexScan` / the translator's index-matching rule).
pub struct IndexScan {
    schema: TableSchema,
    managers: &'static Managers,
    index_id: ContainerId,
    transaction_id: TransactionId,
    key: Vec<Field>,

    open: bool,
    matches: Vec<ValueId>,
    next_idx: usize,
}

impl IndexScan {
    pub fn new(
        managers: &'static Managers,
        schema: &TableSchema,
        index_id: ContainerId,
        tid: TransactionId,
        key: Vec<Field>,
    ) -> Self {
        Self {
            schema: schema.clone(),
            managers,
            index_id,
            transaction_id: tid,
            key,
            open: false,
            matches: Vec::new(),
            next_idx: 0,
        }
    }
}

impl OpIterator for IndexScan {
    fn configure(&mut self, _will_rewind: bool) {
        // no children
    }

    fn open(&mut self) -> Result<(), CrustyError> {
        if !self.open {
            self.matches = self.managers.im.search(self.index_id, &self.key)?;
            self.next_idx = 0;
        }
        self.open = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Tuple>, CrustyError> {
        if !self.open {
            panic!("Operator has not been opened")
        }
        if self.next_idx >= self.matches.len() {
            return Ok(None);
        }
        let id = self.matches[self.next_idx];
        self.next_idx += 1;
        let bytes = self
            .managers
            .sm
            .get_value(id, self.transaction_id, Permissions::ReadOnly)?;
        let mut tuple = Tuple::from_bytes(&bytes);
        tuple.value_id = Some(id);
        Ok(Some(tuple))
    }

    fn close(&mut self) -> Result<(), CrustyError> {
        self.matches.clear();
        self.next_idx = 0;
        self.open = false;
        Ok(())
    }

    fn rewind(&mut self) -> Result<(), CrustyError> {
        if !self.open {
            panic!("Operator has not been opened")
        }
        self.next_idx = 0;
        Ok(())
    }

    fn get_schema(&self) -> &TableSchema {
        &self.schema
    }
}
