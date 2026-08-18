//! A B+Tree secondary/primary index built on top of the existing slotted-page
//! storage engine (`heapstore::page::Page` / `heapstore::heap_page::HeapPage`).
//!
//! Design note: rather than inventing a new on-disk page format for B+Tree
//! nodes, each node is stored as an ordinary `HeapPage`. Entries are appended
//! with `add_value` (O(1), no physical ordering maintained), and logical key
//! order is recovered by reading + in-memory-sorting a node's entries whenever
//! the tree needs to traverse or split it. Node fanout is small enough (bounded
//! by a 4KB page) that this sort is cheap, and it means the B+Tree reuses the
//! already-tested slotted-page insert/delete/compaction logic instead of
//! duplicating it.
//!
//! Page 0 of the container is a header page holding the current root page id.
//! All other pages are B+Tree nodes. The first entry ever added to a node page
//! is always a `NodeEntry::Meta` describing whether it's a leaf and (for
//! leaves) its left/right siblings — leaves form a doubly-linked chain in key
//! order, used for range/duplicate scans.
//!
//! Handling non-unique keys (secondary indexes): a run of many rows sharing
//! the same key can span more than one leaf. Internal-node separator keys
//! alone can't disambiguate exactly which leaf in such a run is the
//! *leftmost* one (the tree descent for a plain key may land anywhere inside
//! the run). Rather than requiring separators to carry a full `(key, rid)`
//! tie-breaker (which pushes the ambiguity into internal nodes instead of
//! resolving it), `search`/`delete` first descend to *some* leaf that could
//! hold the key, then walk left along the leaf chain while the predecessor's
//! last entry still matches, to find the true leftmost leaf of the run —
//! then scan right from there. This is correct regardless of how a given
//! duplicate run happened to be split across leaves.

use common::error::c_err;
use common::prelude::*;
use heapstore::buffer_pool::buffer_frame::{FrameReadGuard, FrameWriteGuard};
use heapstore::buffer_pool::mem_pool_trait::{MemPool, PageFrameId};
use heapstore::heap_page::HeapPage;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

/// A key is a (possibly composite) tuple of field values, compared lexicographically.
pub type IndexKey = Vec<Field>;

/// A comparable projection of `ValueId`, used only to give entries that share
/// the same index key a deterministic total order within a single leaf split
/// (`ValueId` itself derives no `Ord`).
type RidKey = (ContainerId, Option<PageId>, Option<SlotId>);

fn rid_key(v: &ValueId) -> RidKey {
    (v.container_id, v.page_id, v.slot_id)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
enum NodeEntry {
    /// Always the first entry ever added to a node page.
    Meta {
        is_leaf: bool,
        left_sibling: Option<PageId>,
        right_sibling: Option<PageId>,
    },
    /// `key: None` marks the leftmost child pointer of an internal node
    /// (i.e. "everything less than the smallest real key in this node").
    Internal {
        key: Option<IndexKey>,
        child: PageId,
    },
    Leaf {
        key: IndexKey,
        rid: ValueId,
    },
}

fn encode(entry: &NodeEntry) -> Result<Vec<u8>, CrustyError> {
    serde_cbor::to_vec(entry).map_err(|e| c_err(&format!("index entry encode error: {:?}", e)))
}

fn decode(bytes: &[u8]) -> Result<NodeEntry, CrustyError> {
    serde_cbor::from_slice(bytes).map_err(|e| c_err(&format!("index entry decode error: {:?}", e)))
}

/// A B+Tree index over container `c_id`. Structural modifications (inserts
/// that may split a node) are serialized with `struct_lock`; point lookups
/// don't take it and rely on the buffer pool's per-page latches for safety of
/// individual page reads (see plan: simple index-level lock, not latch-crabbing).
pub struct TreeIndex<T: MemPool> {
    c_id: ContainerId,
    bp: Arc<T>,
    struct_lock: RwLock<()>,
}

impl<T: MemPool> TreeIndex<T> {
    fn get_page_for_read(&self, page_id: PageId) -> Result<FrameReadGuard<'_>, CrustyError> {
        self.bp
            .get_page_for_read(PageFrameId::new(self.c_id, page_id))
            .map_err(|e| c_err(&format!("index page read error: {:?}", e)))
    }

    fn get_page_for_write(&self, page_id: PageId) -> Result<FrameWriteGuard<'_>, CrustyError> {
        self.bp
            .get_page_for_write(PageFrameId::new(self.c_id, page_id))
            .map_err(|e| c_err(&format!("index page write error: {:?}", e)))
    }

    /// Create a brand new, empty B+Tree index in container `c_id`.
    pub fn new(c_id: ContainerId, bp: Arc<T>) -> Result<Self, CrustyError> {
        // Page 0: header page storing the current root page id. Allocated
        // first so it is guaranteed to be page 0 (mirrors HeapFile's
        // convention of a fixed, known header page).
        let mut header = bp
            .create_new_page_for_write(c_id)
            .map_err(|e| c_err(&format!("{:?}", e)))?;
        header.init_heap_page();

        // Page 1: the initial (empty) root, a leaf.
        let mut root = bp
            .create_new_page_for_write(c_id)
            .map_err(|e| c_err(&format!("{:?}", e)))?;
        root.init_heap_page();
        root.add_value(&encode(&NodeEntry::Meta {
            is_leaf: true,
            left_sibling: None,
            right_sibling: None,
        })?);
        let root_id = root.get_page_id();
        drop(root);

        header.add_value(&root_id.to_le_bytes());
        drop(header);

        Ok(TreeIndex {
            c_id,
            bp,
            struct_lock: RwLock::new(()),
        })
    }

    /// Load an existing B+Tree index (its pages are already on disk / in the buffer pool).
    pub fn load(c_id: ContainerId, bp: Arc<T>) -> Self {
        TreeIndex {
            c_id,
            bp,
            struct_lock: RwLock::new(()),
        }
    }

    fn get_root_page_id(&self) -> Result<PageId, CrustyError> {
        let page = self.get_page_for_read(0)?;
        let bytes = page
            .get_value(0)
            .ok_or_else(|| c_err("index header missing root pointer"))?;
        Ok(PageId::from_le_bytes(
            bytes.try_into().map_err(|_| c_err("corrupt index header"))?,
        ))
    }

    fn set_root_page_id(&self, new_root: PageId) -> Result<(), CrustyError> {
        let mut page = self.get_page_for_write(0)?;
        page.update_value(0, &new_root.to_le_bytes())
            .ok_or_else(|| c_err("failed to update index root pointer"))?;
        Ok(())
    }

    fn read_is_leaf(page: &heapstore::page::Page) -> Result<bool, CrustyError> {
        for (bytes, _) in page.iter() {
            if let NodeEntry::Meta { is_leaf, .. } = decode(bytes)? {
                return Ok(is_leaf);
            }
        }
        Err(c_err("index node missing meta entry"))
    }

    fn read_internal_entries(
        page: &heapstore::page::Page,
    ) -> Result<Vec<(Option<IndexKey>, PageId)>, CrustyError> {
        let mut entries = Vec::new();
        for (bytes, _) in page.iter() {
            if let NodeEntry::Internal { key, child } = decode(bytes)? {
                entries.push((key, child));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    /// Returns (left_sibling, right_sibling, sorted entries) for a leaf page.
    fn read_leaf_entries(
        page: &heapstore::page::Page,
    ) -> Result<(Option<PageId>, Option<PageId>, Vec<(IndexKey, ValueId)>), CrustyError> {
        let mut left_sibling = None;
        let mut right_sibling = None;
        let mut entries = Vec::new();
        for (bytes, _) in page.iter() {
            match decode(bytes)? {
                NodeEntry::Meta {
                    left_sibling: ls,
                    right_sibling: rs,
                    ..
                } => {
                    left_sibling = ls;
                    right_sibling = rs;
                }
                NodeEntry::Leaf { key, rid } => entries.push((key, rid)),
                NodeEntry::Internal { .. } => return Err(c_err("expected leaf node")),
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| rid_key(&a.1).cmp(&rid_key(&b.1))));
        Ok((left_sibling, right_sibling, entries))
    }

    /// Overwrite a leaf's Meta entry's `left_sibling` pointer, keeping
    /// everything else. Used to re-thread the leaf chain when a split
    /// inserts a new page between two existing siblings.
    fn set_leaf_left_sibling(&self, leaf_id: PageId, new_left: Option<PageId>) -> Result<(), CrustyError> {
        let mut page = self.get_page_for_write(leaf_id)?;
        let mut target = None;
        for (bytes, slot_id) in page.iter() {
            if let NodeEntry::Meta {
                is_leaf,
                right_sibling,
                ..
            } = decode(bytes)?
            {
                target = Some((slot_id, is_leaf, right_sibling));
                break;
            }
        }
        let (slot_id, is_leaf, right_sibling) =
            target.ok_or_else(|| c_err("index node missing meta entry"))?;
        let new_meta = encode(&NodeEntry::Meta {
            is_leaf,
            left_sibling: new_left,
            right_sibling,
        })?;
        page.update_value(slot_id, &new_meta)
            .ok_or_else(|| c_err("failed to re-thread leaf sibling pointer"))?;
        Ok(())
    }

    /// Given an internal node's sorted entries, find the child pointer to follow for `key`.
    /// Ties (multiple separators equal to `key`, from a duplicate-key run that
    /// spans several leaves) route to the *last* matching entry; `search`/
    /// `delete` correct for this afterward by walking left along the leaf chain.
    fn find_child(entries: &[(Option<IndexKey>, PageId)], key: &[Field]) -> PageId {
        let mut result = entries[0].1;
        for (k, child) in entries {
            match k {
                None => result = *child,
                Some(k) if k.as_slice() <= key => result = *child,
                _ => break,
            }
        }
        result
    }

    fn find_leaf(&self, page_id: PageId, key: &[Field]) -> Result<PageId, CrustyError> {
        let (is_leaf, child) = {
            let page = self.get_page_for_read(page_id)?;
            if Self::read_is_leaf(&page)? {
                (true, None)
            } else {
                let entries = Self::read_internal_entries(&page)?;
                (false, Some(Self::find_child(&entries, key)))
            }
        };
        if is_leaf {
            Ok(page_id)
        } else {
            self.find_leaf(child.unwrap(), key)
        }
    }

    /// Starting from some leaf that could hold `key`, walk left along the
    /// leaf chain to find the true leftmost leaf of the (possibly
    /// multi-leaf) run of entries matching `key`.
    fn leftmost_run_leaf(&self, start: PageId, key: &[Field]) -> Result<PageId, CrustyError> {
        let mut current = start;
        loop {
            let left_sibling = {
                let page = self.get_page_for_read(current)?;
                Self::read_leaf_entries(&page)?.0
            };
            match left_sibling {
                Some(prev_id) => {
                    let prev_matches = {
                        let prev_page = self.get_page_for_read(prev_id)?;
                        let (_, _, prev_entries) = Self::read_leaf_entries(&prev_page)?;
                        prev_entries.last().map(|(k, _)| k.as_slice() == key).unwrap_or(false)
                    };
                    if prev_matches {
                        current = prev_id;
                    } else {
                        return Ok(current);
                    }
                }
                None => return Ok(current),
            }
        }
    }

    /// Point lookup: returns every `ValueId` stored under `key` (more than one
    /// only for non-unique/secondary indexes with duplicate keys).
    pub fn search(&self, key: &[Field]) -> Result<Vec<ValueId>, CrustyError> {
        let _guard = self.struct_lock.read().unwrap();
        let root_id = self.get_root_page_id()?;
        let landed = self.find_leaf(root_id, key)?;
        let mut current = Some(self.leftmost_run_leaf(landed, key)?);
        let mut results = Vec::new();
        while let Some(page_id) = current {
            let page = self.get_page_for_read(page_id)?;
            let (_, right_sibling, entries) = Self::read_leaf_entries(&page)?;
            for (k, rid) in &entries {
                if k.as_slice() == key {
                    results.push(*rid);
                }
            }
            // Duplicates of `key` can only spill onto the right sibling if the
            // last entry on this page still equals `key`.
            current = if entries.last().map(|(k, _)| k.as_slice() == key).unwrap_or(false) {
                right_sibling
            } else {
                None
            };
        }
        Ok(results)
    }

    /// Insert `(key, rid)` into the tree.
    pub fn insert(&self, key: IndexKey, rid: ValueId) -> Result<(), CrustyError> {
        let _guard = self.struct_lock.write().unwrap();
        let root_id = self.get_root_page_id()?;
        if let Some((sep_key, new_page_id)) = self.insert_recursive(root_id, &key, rid)? {
            // The root split: build a brand new root with two children.
            let mut new_root = self
                .bp
                .create_new_page_for_write(self.c_id)
                .map_err(|e| c_err(&format!("{:?}", e)))?;
            new_root.init_heap_page();
            new_root.add_value(&encode(&NodeEntry::Meta {
                is_leaf: false,
                left_sibling: None,
                right_sibling: None,
            })?);
            new_root.add_value(&encode(&NodeEntry::Internal {
                key: None,
                child: root_id,
            })?);
            new_root.add_value(&encode(&NodeEntry::Internal {
                key: Some(sep_key),
                child: new_page_id,
            })?);
            let new_root_id = new_root.get_page_id();
            drop(new_root);
            self.set_root_page_id(new_root_id)?;
        }
        Ok(())
    }

    /// Returns `Some((separator_key, new_right_sibling_page_id))` if `page_id` split.
    fn insert_recursive(
        &self,
        page_id: PageId,
        key: &IndexKey,
        rid: ValueId,
    ) -> Result<Option<(IndexKey, PageId)>, CrustyError> {
        let is_leaf = {
            let page = self.get_page_for_read(page_id)?;
            Self::read_is_leaf(&page)?
        };
        if is_leaf {
            self.insert_into_leaf(page_id, key, rid)
        } else {
            let child_id = {
                let page = self.get_page_for_read(page_id)?;
                let entries = Self::read_internal_entries(&page)?;
                Self::find_child(&entries, key)
            };
            match self.insert_recursive(child_id, key, rid)? {
                Some((sep_key, new_child_id)) => {
                    self.insert_into_internal(page_id, sep_key, new_child_id)
                }
                None => Ok(None),
            }
        }
    }

    fn insert_into_leaf(
        &self,
        page_id: PageId,
        key: &IndexKey,
        rid: ValueId,
    ) -> Result<Option<(IndexKey, PageId)>, CrustyError> {
        let mut page = self.get_page_for_write(page_id)?;
        let entry_bytes = encode(&NodeEntry::Leaf {
            key: key.clone(),
            rid,
        })?;
        if page.add_value(&entry_bytes).is_some() {
            return Ok(None);
        }

        // Full: split. Gather all live entries (draining the page), redistribute.
        // Sorting by (key, rid) — not just key — gives every entry a well
        // defined position even within a run of duplicate keys, so the split
        // point is always well defined (see module docs for how the search
        // path compensates for a run landing across the resulting boundary).
        let (left_sibling, old_right_sibling, mut entries) = Self::drain_leaf(&mut page)?;
        entries.push((key.clone(), rid));
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| rid_key(&a.1).cmp(&rid_key(&b.1))));
        let mid = entries.len() / 2;
        let (left_entries, right_entries) = entries.split_at(mid);
        let separator = right_entries[0].0.clone();

        let mut right_page = self
            .bp
            .create_new_page_for_write(self.c_id)
            .map_err(|e| c_err(&format!("{:?}", e)))?;
        right_page.init_heap_page();
        right_page.add_value(&encode(&NodeEntry::Meta {
            is_leaf: true,
            left_sibling: Some(page_id),
            right_sibling: old_right_sibling,
        })?);
        for (k, v) in right_entries {
            right_page.add_value(&encode(&NodeEntry::Leaf {
                key: k.clone(),
                rid: *v,
            })?);
        }
        let right_page_id = right_page.get_page_id();
        drop(right_page);

        page.add_value(&encode(&NodeEntry::Meta {
            is_leaf: true,
            left_sibling,
            right_sibling: Some(right_page_id),
        })?);
        for (k, v) in left_entries {
            page.add_value(&encode(&NodeEntry::Leaf {
                key: k.clone(),
                rid: *v,
            })?);
        }
        // The write guard on `page_id` must be dropped before re-latching it
        // indirectly via `set_leaf_left_sibling` below (only relevant when
        // `old_right_sibling` is Some, i.e. `page_id` wasn't the rightmost leaf).
        drop(page);

        if let Some(old_right_id) = old_right_sibling {
            self.set_leaf_left_sibling(old_right_id, Some(right_page_id))?;
        }

        Ok(Some((separator, right_page_id)))
    }

    fn insert_into_internal(
        &self,
        page_id: PageId,
        sep_key: IndexKey,
        new_child: PageId,
    ) -> Result<Option<(IndexKey, PageId)>, CrustyError> {
        let mut page = self.get_page_for_write(page_id)?;
        let entry_bytes = encode(&NodeEntry::Internal {
            key: Some(sep_key.clone()),
            child: new_child,
        })?;
        if page.add_value(&entry_bytes).is_some() {
            return Ok(None);
        }

        // Full: split. The middle entry's key is pushed up to the parent and
        // removed from both children; its child pointer becomes the new right
        // node's leftmost (key = None) pointer.
        let mut entries = Self::drain_internal(&mut page)?;
        entries.push((Some(sep_key), new_child));
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mid = entries.len() / 2;
        let left_entries = &entries[..mid];
        let mid_entry = &entries[mid];
        let right_rest = &entries[mid + 1..];
        let separator = mid_entry
            .0
            .clone()
            .ok_or_else(|| c_err("internal split picked the leftmost sentinel as separator"))?;

        let mut right_page = self
            .bp
            .create_new_page_for_write(self.c_id)
            .map_err(|e| c_err(&format!("{:?}", e)))?;
        right_page.init_heap_page();
        right_page.add_value(&encode(&NodeEntry::Meta {
            is_leaf: false,
            left_sibling: None,
            right_sibling: None,
        })?);
        right_page.add_value(&encode(&NodeEntry::Internal {
            key: None,
            child: mid_entry.1,
        })?);
        for (k, c) in right_rest {
            right_page.add_value(&encode(&NodeEntry::Internal {
                key: k.clone(),
                child: *c,
            })?);
        }
        let right_page_id = right_page.get_page_id();
        drop(right_page);

        page.add_value(&encode(&NodeEntry::Meta {
            is_leaf: false,
            left_sibling: None,
            right_sibling: None,
        })?);
        for (k, c) in left_entries {
            page.add_value(&encode(&NodeEntry::Internal {
                key: k.clone(),
                child: *c,
            })?);
        }

        Ok(Some((separator, right_page_id)))
    }

    /// Remove every entry from a leaf page (leaving the Meta entry gone too),
    /// returning its (left_sibling, right_sibling) and the drained (key, rid) pairs.
    fn drain_leaf(
        page: &mut FrameWriteGuard,
    ) -> Result<(Option<PageId>, Option<PageId>, Vec<(IndexKey, ValueId)>), CrustyError> {
        let mut left_sibling = None;
        let mut right_sibling = None;
        let mut entries = Vec::new();
        let mut slots = Vec::new();
        for (bytes, slot_id) in page.iter() {
            match decode(bytes)? {
                NodeEntry::Meta {
                    left_sibling: ls,
                    right_sibling: rs,
                    ..
                } => {
                    left_sibling = ls;
                    right_sibling = rs;
                }
                NodeEntry::Leaf { key, rid } => entries.push((key, rid)),
                NodeEntry::Internal { .. } => return Err(c_err("expected leaf node")),
            }
            slots.push(slot_id);
        }
        for slot_id in slots {
            page.delete_value(slot_id);
        }
        Ok((left_sibling, right_sibling, entries))
    }

    fn drain_internal(
        page: &mut FrameWriteGuard,
    ) -> Result<Vec<(Option<IndexKey>, PageId)>, CrustyError> {
        let mut entries = Vec::new();
        let mut slots = Vec::new();
        for (bytes, slot_id) in page.iter() {
            if let NodeEntry::Internal { key, child } = decode(bytes)? {
                entries.push((key, child));
            }
            slots.push(slot_id);
        }
        for slot_id in slots {
            page.delete_value(slot_id);
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    /// Remove one `(key, rid)` entry. No merge-on-underflow: a sparse leaf
    /// remains structurally valid, just less space-efficient (documented v1
    /// limitation).
    pub fn delete(&self, key: &[Field], rid: ValueId) -> Result<(), CrustyError> {
        let _guard = self.struct_lock.write().unwrap();
        let root_id = self.get_root_page_id()?;
        let landed = self.find_leaf(root_id, key)?;
        let mut current = self.leftmost_run_leaf(landed, key)?;
        loop {
            let (found_slot, last_matches, right_sibling) = {
                let page = self.get_page_for_read(current)?;
                let mut found_slot = None;
                for (bytes, slot_id) in page.iter() {
                    if let NodeEntry::Leaf { key: k, rid: r } = decode(bytes)? {
                        if k == key && r == rid {
                            found_slot = Some(slot_id);
                            break;
                        }
                    }
                }
                let (_, right_sibling, entries) = Self::read_leaf_entries(&page)?;
                let last_matches =
                    entries.last().map(|(k, _)| k.as_slice() == key).unwrap_or(false);
                (found_slot, last_matches, right_sibling)
            };
            if let Some(slot_id) = found_slot {
                let mut page = self.get_page_for_write(current)?;
                page.delete_value(slot_id);
                return Ok(());
            }
            match (last_matches, right_sibling) {
                (true, Some(next)) => current = next,
                _ => return Ok(()), // not found; nothing to delete
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heapstore::buffer_pool::buffer_pool::{gen_random_pathname, BufferPool};
    use heapstore::container_file_catalog::ContainerFileCatalog;

    fn new_test_tree() -> TreeIndex<BufferPool> {
        let temp_dir = std::env::temp_dir();
        let dir = temp_dir.join(gen_random_pathname(Some("btree_test")));
        let cfc = Arc::new(ContainerFileCatalog::new(dir, true).unwrap());
        let bp = Arc::new(BufferPool::new(500, cfc).unwrap());
        TreeIndex::new(1, bp).unwrap()
    }

    fn k(v: i64) -> IndexKey {
        vec![Field::BigInt(v)]
    }

    #[test]
    fn insert_and_search_single_key() {
        let tree = new_test_tree();
        tree.insert(k(5), ValueId::new_slot(1, 1, 0)).unwrap();
        let res = tree.search(&k(5)).unwrap();
        assert_eq!(res, vec![ValueId::new_slot(1, 1, 0)]);
        assert!(tree.search(&k(6)).unwrap().is_empty());
    }

    #[test]
    fn insert_many_forces_splits_and_all_are_findable() {
        let tree = new_test_tree();
        let n = 2000;
        for i in 0..n {
            tree.insert(k(i), ValueId::new_slot(1, (i % 500) as u32, (i % 100) as u16))
                .unwrap();
        }
        for i in 0..n {
            let res = tree.search(&k(i)).unwrap();
            assert_eq!(res.len(), 1, "missing key {}", i);
            assert_eq!(
                res[0],
                ValueId::new_slot(1, (i % 500) as u32, (i % 100) as u16)
            );
        }
        assert!(tree.search(&k(n + 1)).unwrap().is_empty());
    }

    #[test]
    fn non_unique_secondary_index_duplicates() {
        let tree = new_test_tree();
        // Many rows share the same key, as in a non-unique secondary index.
        for i in 0..300 {
            tree.insert(k(42), ValueId::new_slot(1, i, 0)).unwrap();
        }
        let res = tree.search(&k(42)).unwrap();
        assert_eq!(res.len(), 300);
    }

    #[test]
    fn many_distinct_duplicate_runs_interleaved() {
        // Rows for several distinct keys, each with several duplicates,
        // inserted in an interleaved (round-robin) order so runs are more
        // likely to end up split across non-adjacent-looking leaves.
        let tree = new_test_tree();
        let keys = [1i64, 2, 3, 4, 5];
        let per_key = 150;
        for round in 0..per_key {
            for &key in &keys {
                tree.insert(k(key), ValueId::new_slot(1, (key * 10000 + round) as u32, 0))
                    .unwrap();
            }
        }
        for &key in &keys {
            let res = tree.search(&k(key)).unwrap();
            assert_eq!(res.len(), per_key as usize, "key {} count mismatch", key);
        }
    }

    #[test]
    fn delete_removes_entry() {
        let tree = new_test_tree();
        for i in 0..500 {
            tree.insert(k(i), ValueId::new_slot(1, i as u32, 0)).unwrap();
        }
        tree.delete(&k(250), ValueId::new_slot(1, 250, 0)).unwrap();
        assert!(tree.search(&k(250)).unwrap().is_empty());
        // Neighbors untouched.
        assert_eq!(tree.search(&k(249)).unwrap().len(), 1);
        assert_eq!(tree.search(&k(251)).unwrap().len(), 1);
    }

    #[test]
    fn delete_one_of_many_duplicates() {
        let tree = new_test_tree();
        let rids: Vec<ValueId> = (0..300).map(|i| ValueId::new_slot(1, i, 0)).collect();
        for &rid in &rids {
            tree.insert(k(42), rid).unwrap();
        }
        tree.delete(&k(42), rids[150]).unwrap();
        let res = tree.search(&k(42)).unwrap();
        assert_eq!(res.len(), 299);
        assert!(!res.contains(&rids[150]));
    }

    #[test]
    fn composite_key_lexicographic_ordering() {
        let tree = new_test_tree();
        let key_a = vec![Field::BigInt(1), Field::String("a".to_string())];
        let key_b = vec![Field::BigInt(1), Field::String("b".to_string())];
        let key_c = vec![Field::BigInt(2), Field::String("a".to_string())];
        tree.insert(key_a.clone(), ValueId::new_slot(1, 1, 0)).unwrap();
        tree.insert(key_b.clone(), ValueId::new_slot(1, 2, 0)).unwrap();
        tree.insert(key_c.clone(), ValueId::new_slot(1, 3, 0)).unwrap();
        assert_eq!(tree.search(&key_a).unwrap(), vec![ValueId::new_slot(1, 1, 0)]);
        assert_eq!(tree.search(&key_b).unwrap(), vec![ValueId::new_slot(1, 2, 0)]);
        assert_eq!(tree.search(&key_c).unwrap(), vec![ValueId::new_slot(1, 3, 0)]);
    }
}
