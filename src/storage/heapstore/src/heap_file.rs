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

        // Add any extra initialization code in this function.

        let heap_file = HeapFile {
            c_id,
            bp: mem_pool.clone(),
        };
        Ok(heap_file)
    }

    /// Load an existing heap file.
    pub fn load(c_id: ContainerId, mem_pool: Arc<T>) -> Result<Self, CrustyError> {
        // Add any extra initialization code in this function.

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
        let page = self.get_page_for_read(page_id);
        panic!("TODO milestone hs");
    }

    // Delete a value at (page_id, slot_id) from the heap file.
    pub fn delete_val(&self, page_id: PageId, slot_id: SlotId) -> Result<(), CrustyError> {
        let mut page = self.get_page_for_write(page_id);
        panic!("TODO milestone hs");
    }

    pub fn update_val(
        &self,
        page_id: PageId,
        slot_id: SlotId,
        val: &[u8],
    ) -> Result<ValueId, CrustyError> {
        let mut page = self.get_page_for_write(page_id);
        panic!("TODO milestone hs");
    }

    // This function is not implemented in a thread-safe way. Can cause deadlocks when used in a multi-threaded environment.
    // We do not care about this for now.
    pub fn add_val(&self, val: &[u8]) -> Result<ValueId, CrustyError> {
        panic!("TODO milestone hs");
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
        // Create the HeapFileIter
        panic!("TODO milestone hs");
    }

    pub fn iter_from(self: &Arc<Self>, page_id: PageId, slot_id: SlotId) -> HeapFileIter<T> {
        // Create the HeapFileIter
        panic!("TODO milestone hs");
    }
}

pub struct HeapFileIter<T: MemPool> {
    /// We are providing the elements of the iterator that we used, you are allowed to
    /// use them in the iterator or make changes. If you change the elements, you
    /// will want to change the new_from constructor to use the new elements.
    heapfile: Arc<HeapFile<T>>,
    initialized: bool,
    finished: bool,
    first_page: PageId,
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
        // If any work is needed to be done to initialize the iterator, do it here.
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

        // Implement the iterator logic
        panic!("TODO milestone hs");
    }
}
