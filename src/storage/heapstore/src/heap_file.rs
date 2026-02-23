use crate::buffer_pool::buffer_frame::FrameReadGuard;
use crate::buffer_pool::buffer_frame::FrameWriteGuard;
use crate::buffer_pool::mem_pool_trait::MemPool;
use crate::buffer_pool::mem_pool_trait::PageFrameId;
use crate::heap_page::HeapPage;
use common::error::c_err;
#[allow(unused_imports)]
use common::ids::AtomicPageId;
use common::prelude::*;
#[allow(unused_imports)]
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The struct for a heap file.  
pub(crate) struct HeapFile<T: MemPool> {
    c_id: ContainerId,
    bp: Arc<T>,
}

/// HeapFile required functions
impl<T: MemPool> HeapFile<T> {
    /// Helper function to fetch a page for read from the buffer pool.
    fn get_page_for_read(&self, page_id: PageId) -> FrameReadGuard<'_> {
        self.bp
            .get_page_for_read(PageFrameId::new(self.c_id, page_id))
            .unwrap()
    }

    /// Helper function to fetch a page for write from the buffer pool.
    fn get_page_for_write(&self, page_id: PageId) -> FrameWriteGuard<'_> {
        self.bp
            .get_page_for_write(PageFrameId::new(self.c_id, page_id))
            .unwrap()
    }

    /// Create a brand-new heap file for container `c_id`.
    pub fn new(c_id: ContainerId, mem_pool: Arc<T>) -> Result<Self, CrustyError> {
        // Note that the header page is always page 0, and the data pages start from 1.
        // You may not end up using the header page, but some tests will assume this.

        let heap_file = HeapFile {
            c_id,
            bp: mem_pool.clone(),
        };
        // Allocate page 0 as the header page. Content is left zeroed.
        // Dropping the guard marks it dirty so the buffer pool persists it on eviction.
        let _header = mem_pool
            .create_new_page_for_write(c_id)
            .map_err(|e| c_err(&format!("{:?}", e)))?;
        Ok(heap_file)
    }

    /// Load an existing heap file.
    pub fn load(c_id: ContainerId, mem_pool: Arc<T>) -> Result<Self, CrustyError> {
        // Add any extra initialization code in this function.

        // Maybe this implementation?
        // Ok(HeapFile {
        //     c_id,
        //     bp: mem_pool,
        // })

        // OG implementation
        let heap_file = HeapFile {
            c_id,
            bp: mem_pool.clone(),
        };
        Ok(heap_file)
    }

    /// Return the number of pages for this HeapFile.
    /// Return type is PageId (alias for another type) as we cannot have more
    /// pages than PageId can hold.
    pub fn num_pages(&self) -> PageId {
        self.bp.get_max_page_id(self.c_id).unwrap_or(0)
    }

    /// Read a value at (page_id, slot_id) from the heap file.
    pub fn get_val(&self, page_id: PageId, slot_id: SlotId) -> Result<Vec<u8>, CrustyError> {
        if page_id >= self.num_pages() {
            return Err(c_err("Page not found"));
        }
        let page = self.get_page_for_read(page_id);
        page.get_value(slot_id)
            .map(|v| v.to_vec())
            .ok_or_else(|| c_err("Value not found"))
    }

    // Delete a value at (page_id, slot_id) from the heap file.
    pub fn delete_val(&self, page_id: PageId, slot_id: SlotId) -> Result<(), CrustyError> {
        if page_id >= self.num_pages() {
            return Err(c_err("Page not found"));
        }
        let mut page = self.get_page_for_write(page_id);
        page.delete_value(slot_id)
            .ok_or_else(|| c_err("Value not found or already deleted"))
    }

    pub fn update_val(
        &self,
        page_id: PageId,
        slot_id: SlotId,
        val: &[u8],
    ) -> Result<ValueId, CrustyError> {
        if page_id >= self.num_pages() {
            return Err(c_err("Page not found"));
        }
        {
            let mut page = self.get_page_for_write(page_id);
            if page.update_value(slot_id, val).is_some() {
                return Ok(ValueId::new_slot(self.c_id, page_id, slot_id));
            }
            // Distinguish "slot not found / deleted" from "no space on page"
            if page.get_value(slot_id).is_none() {
                return Err(c_err("Update failed: slot not found"));
            }
            // Slot exists but value doesn't fit in place — delete to free space
            page.delete_value(slot_id)
                .ok_or_else(|| c_err("Delete failed during update fallback"))?;
        } // write guard dropped here — page latch released before add_val
        // Re-insert on any page that has room (may return a different ValueId)
        self.add_val(val)
    }

    // This function is not implemented in a thread-safe way. Can cause deadlocks when used in a multi-threaded environment.
    // We do not care about this for now.
    pub fn add_val(&self, val: &[u8]) -> Result<ValueId, CrustyError> {
        // Linear scan of existing data pages
        // (page 0 is the header; data starts at 1).
        //
        // Obvious optimizations are possible, like adding indexing.
        // Currently, it is not needed
        for page_id in 1..self.num_pages() {
            let mut page = self.get_page_for_write(page_id);
            if let Some(slot_id) = page.add_value(val) {
                return Ok(ValueId::new_slot(self.c_id, page_id, slot_id));
            }
        }
        // All existing pages are full (or no data pages exist yet) — allocate a new one.
        let mut new_page = self
            .bp
            .create_new_page_for_write(self.c_id)
            .map_err(|e| c_err(&format!("{:?}", e)))?;
        new_page.init_heap_page();
        let page_id = new_page.get_page_id();
        let slot_id = new_page
            .add_value(val)
            .ok_or_else(|| c_err("Value too large to fit in a single page"))?;
        Ok(ValueId::new_slot(self.c_id, page_id, slot_id))
    }

    pub fn add_vals(
        &self,
        iter: impl Iterator<Item = Vec<u8>>,
    ) -> Result<Vec<ValueId>, CrustyError> {
        // You can change this function if desired.
        let mut val_ids = Vec::new();
        for val in iter {
            let val_id = self.add_val(&val)?;
            val_ids.push(val_id);
        }
        Ok(val_ids)
    }

    pub fn iter(self: &Arc<Self>) -> HeapFileIter<T> {
        HeapFileIter::new_from(self.clone(), 1, 0)
    }

    pub fn iter_from(self: &Arc<Self>, page_id: PageId, slot_id: SlotId) -> HeapFileIter<T> {
        HeapFileIter::new_from(self.clone(), page_id, slot_id)
    }
}

pub struct HeapFileIter<T: MemPool> {
    heapfile: Arc<HeapFile<T>>,
    initialized: bool,
    finished: bool,
    first_page: PageId,
    current_page_id: PageId,
    current_slot_id: SlotId,
    current_page: Option<FrameReadGuard<'static>>,
}

impl<T: MemPool> HeapFileIter<T> {
    fn new_from(heapfile: Arc<HeapFile<T>>, page_id: PageId, slot_id: SlotId) -> Self {
        HeapFileIter {
            heapfile,
            initialized: false,
            finished: false,
            first_page: page_id,
            current_page_id: page_id,
            current_slot_id: slot_id,
            current_page: None,
        }
    }

    // Helper function to get a page for read from the buffer pool.
    fn get_page(&self, page_id: PageId) -> FrameReadGuard<'static> {
        // Safety: self.heapfile object has a reference to the buffer pool
        // which makes sure that the frame is not deallocated while this
        // (self) object is alive.
        let page = self.heapfile.get_page_for_read(page_id);
        unsafe { std::mem::transmute::<FrameReadGuard, FrameReadGuard<'static>>(page) }
    }

    fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        if self.first_page < self.heapfile.num_pages() {
            self.current_page = Some(self.get_page(self.first_page));
        } else {
            self.finished = true;
        }
        self.initialized = true;
    }
}

impl<T: MemPool> Iterator for HeapFileIter<T> {
    type Item = (Vec<u8>, ValueId);

    /// This function is called to get the next element of the iterator.
    /// It should return None when the iterator is finished.
    /// Otherwise it should return Some((val, val_id)).
    /// The val is the value that was read from the heap file.
    /// The val_id is the ValueId that was read from the heap file.
    fn next(&mut self) -> Option<Self::Item> {
        // Initialize the iterator
        if !self.initialized {
            self.initialize();
        }

        if self.finished {
            return None;
        }
        loop {
            if self.current_page.is_none() {
                return None;
            }
            // u16 is Copy — borrow of current_page ends immediately
            let num_slots = self.current_page.as_ref().unwrap().get_num_slots();

            if self.current_slot_id < num_slots {
                let slot_id = self.current_slot_id;
                self.current_slot_id += 1;
                // .to_vec() produces owned Vec<u8>; borrow of current_page ends here
                let val_opt = self
                    .current_page
                    .as_ref()
                    .unwrap()
                    .get_value(slot_id)
                    .map(|v| v.to_vec());
                if let Some(val) = val_opt {
                    return Some((
                        val,
                        ValueId::new_slot(self.heapfile.c_id, self.current_page_id, slot_id),
                    ));
                }
                // Deleted slot — continue to next slot_id
            } else {
                // Page exhausted — advance to next data page
                self.current_slot_id = 0;
                self.current_page = None; // drop FrameReadGuard, releases read latch
                self.current_page_id += 1;
                if self.current_page_id >= self.heapfile.num_pages() {
                    self.finished = true;
                    return None;
                }
                self.current_page = Some(self.get_page(self.current_page_id));
            }
        }
    }
}
