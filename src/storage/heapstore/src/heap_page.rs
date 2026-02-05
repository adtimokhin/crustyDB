use common::prelude::*;
#[allow(unused_imports)]
use common::PAGE_SIZE;

#[allow(unused_imports)]
use crate::page::{Offset, Page, OFFSET_NUM_BYTES, PAGE_FIXED_HEADER_LEN};

use std::mem;

#[allow(dead_code)]
/// The size of a slotID
pub(crate) const SLOT_ID_SIZE: usize = mem::size_of::<SlotId>();
#[allow(dead_code)]
/// The allowed metadata size per slot
pub(crate) const SLOT_METADATA_SIZE: usize = 4;
#[allow(dead_code)]
/// The size of the metadata allowed for the heap page, this is in addition to the page header
pub(crate) const HEAP_PAGE_FIXED_METADATA_SIZE: usize = 8;

/// Offset of the slot count within the heap metadata (relative to Deref start, i.e. after PAGE_FIXED_HEADER_LEN).
/// Stored as a u16 (2 bytes).
const NUM_SLOTS_OFFSET: usize = 0;
const NUM_SLOTS_SIZE: usize = mem::size_of::<u16>();

/// Offset of the free space pointer within the heap metadata (relative to Deref start).
/// Points to the start of free space in the data area. Stored as a u16 (2 bytes).
const FREE_SPACE_PTR_OFFSET: usize = NUM_SLOTS_OFFSET + NUM_SLOTS_SIZE;
const FREE_SPACE_PTR_SIZE: usize = mem::size_of::<u16>();

/// Total bytes of data in deleted (hole) regions. Stored as a u16 (2 bytes).
const DELETED_BYTES_OFFSET: usize = FREE_SPACE_PTR_OFFSET + FREE_SPACE_PTR_SIZE;
const DELETED_BYTES_SIZE: usize = mem::size_of::<u16>();

/// Number of slots currently marked as deleted. Stored as a u16 (2 bytes).
const DELETED_SLOT_COUNT_OFFSET: usize = DELETED_BYTES_OFFSET + DELETED_BYTES_SIZE;
const DELETED_SLOT_COUNT_SIZE: usize = mem::size_of::<u16>();

/// This is trait of a HeapPage for the Page struct.
///
/// The page header size is fixed to `PAGE_FIXED_HEADER_LEN` bytes and you will use
/// additional bytes for the HeapPage metadata
/// Your HeapPage implementation can use a fixed metadata of 8 bytes plus 4 bytes per value/entry/slot stored.
/// For example a page that has stored 3 values, we would assume that the fist
/// `PAGE_FIXED_HEADER_LEN` bytes are used for the page metadata, 8 bytes for the HeapPage metadata
/// and 12 bytes for slot meta data (4 bytes for each of the 3 values).
/// This leave the rest free for storing data (PAGE_SIZE-PAGE_FIXED_HEADER_LEN-8-12).
///
/// If you delete a value, you do not need reclaim header space the way you must reclaim page
/// body space. E.g., if you insert 3 values then delete 2 of them, your header can remain 26
/// bytes & subsequent inserts can simply add 6 more bytes to the header as normal.
/// The rest must filled as much as possible to hold values.
pub trait HeapPage {
    ////////////////////////////////////////////////////////////////////////////
    ///                         Helper Functions
    ////////////////////////////////////////////////////////////////////////////
    /// Get the total number of slots (active and deleted) on this page.
    fn get_num_slots(&self) -> u16;

    /// Set the total number of slots (active and deleted) on this page.
    fn set_num_slots(&mut self, num_slots: u16);

    /// Increment the total number of slots by 1 and return the new count.
    fn increment_num_slots(&mut self) -> u16;

    /// Decrement the total number of slots by 1 and return the new count.
    /// Panics if the slot count is already 0.
    fn decrement_num_slots(&mut self) -> u16;

    /// Get the free space pointer. This points to the start of free space in the data area
    /// (relative to the Deref start).
    fn get_free_space_ptr(&self) -> u16;

    /// Set the free space pointer.
    fn set_free_space_ptr(&mut self, ptr: u16);

    /// Find the next free slot by scanning slot metadata from PAGE_FIXED_HEADER_LEN
    /// to get_header_size(). Returns the offset of the first slot whose length is zero,
    /// or the offset following get_header_size() if all slots are occupied.
    fn find_next_free_slot(&self) -> usize;

    /// Get the total number of bytes in deleted (hole) data regions.
    fn get_deleted_bytes(&self) -> u16;

    /// Set the total number of bytes in deleted (hole) data regions.
    fn set_deleted_bytes(&mut self, bytes: u16);

    /// Get the number of slots currently marked as deleted.
    fn get_deleted_slot_count(&self) -> u16;

    /// Set the number of slots currently marked as deleted.
    fn set_deleted_slot_count(&mut self, count: u16);

    /// Given a slot_id, return the byte offset of that slot's metadata within the page.
    /// Returns -1 if the slot_id is out of range (>= number of slots).
    fn get_slot_metadata_offset(&self, slot_id: SlotId) -> isize;

    /// Compact the page by repacking all live data contiguously from PAGE_SIZE backward,
    /// eliminating holes left by deleted values. Updates slot metadata offsets and
    /// resets deleted_bytes to 0.
    fn compact_page(&mut self);

    // Do not change these functions signatures (only the function bodies)

    /// Initialize the page struct as a heap page.
    #[allow(dead_code)]
    fn init_heap_page(&mut self);

    /// Attempts to add a new value to this page if there is space available.
    /// Returns Some(SlotId) if it was inserted or None if there was not enough space.
    /// Note that where the bytes are stored in the page does not matter (heap), but it
    /// should not change the slotId for any existing value. This means that
    /// bytes in the page may not follow the slot order.
    /// If a slot is deleted you should reuse the slotId in the future.
    /// The page should always assign the lowest available slot_id to an insertion.
    ///
    /// HINT: You can copy/clone bytes into a slice using the following function.
    /// They must have the same size.
    /// self.data[X..y].clone_from_slice(&bytes);
    #[allow(dead_code)]
    fn add_value(&mut self, bytes: &[u8]) -> Option<SlotId>;

    /// Return the bytes for the slotId. If the slotId is not valid then return None
    #[allow(dead_code)]
    fn get_value(&self, slot_id: SlotId) -> Option<&[u8]>;

    /// Delete the bytes/slot for the slotId. If the slotId is not valid then return None
    /// The slotId for a deleted slot should be assigned to the next added value
    /// The space for the value should be free to use for a later added value.
    /// HINT: Return Some(()) for a valid delete
    #[allow(dead_code)]
    fn delete_value(&mut self, slot_id: SlotId) -> Option<()>;

    /// Update the value for the slotId. If the slotId is not valid or there is not
    /// space on the page return None and leave the old value/slot. If there is space, update the value and return Some(())
    #[allow(dead_code)]
    fn update_value(&mut self, slot_id: SlotId, bytes: &[u8]) -> Option<()>;

    /// A utility function to determine the current size of the header for this page
    /// Will be used by tests. Optional for you to use in your code
    #[allow(dead_code)]
    fn get_header_size(&self) -> usize;

    /// A utility function to determine the total current free space in the page.
    /// This should account for the header space used and space that could be reclaimed if needed.
    /// Will be used by tests. Optional for you to use in your code, but strongly suggested
    #[allow(dead_code)]
    fn get_free_space(&self) -> usize;

    #[allow(dead_code)]
    /// Create an iterator for the page. This should return an iterator that will
    /// return the bytes and the slotId for each value in the page.
    fn iter(&self) -> HeapPageIter<'_>;
}

impl HeapPage for Page {
    ////////////////////////////////////////////////////////////////////////////
    ///                         Helper Functions
    ////////////////////////////////////////////////////////////////////////////
    fn get_num_slots(&self) -> u16 {
        u16::from_le_bytes(self[NUM_SLOTS_OFFSET..NUM_SLOTS_OFFSET + NUM_SLOTS_SIZE].try_into().unwrap())
    }

    fn set_num_slots(&mut self, num_slots: u16) {
        self[NUM_SLOTS_OFFSET..NUM_SLOTS_OFFSET + NUM_SLOTS_SIZE]
            .copy_from_slice(&num_slots.to_le_bytes());
    }

    fn increment_num_slots(&mut self) -> u16 {
        let new_count = self.get_num_slots() + 1;
        self.set_num_slots(new_count);
        new_count
    }

    fn decrement_num_slots(&mut self) -> u16 {
        let current = self.get_num_slots();
        assert!(current > 0, "Cannot decrement slot count below 0");
        let new_count = current - 1;
        self.set_num_slots(new_count);
        new_count
    }

    fn get_free_space_ptr(&self) -> u16 {
        u16::from_le_bytes(self[FREE_SPACE_PTR_OFFSET..FREE_SPACE_PTR_OFFSET + FREE_SPACE_PTR_SIZE].try_into().unwrap())
    }

    fn set_free_space_ptr(&mut self, ptr: u16) {
        self[FREE_SPACE_PTR_OFFSET..FREE_SPACE_PTR_OFFSET + FREE_SPACE_PTR_SIZE]
            .copy_from_slice(&ptr.to_le_bytes());
    }

    fn get_deleted_bytes(&self) -> u16 {
        u16::from_le_bytes(self[DELETED_BYTES_OFFSET..DELETED_BYTES_OFFSET + DELETED_BYTES_SIZE].try_into().unwrap())
    }

    fn set_deleted_bytes(&mut self, bytes: u16) {
        self[DELETED_BYTES_OFFSET..DELETED_BYTES_OFFSET + DELETED_BYTES_SIZE]
            .copy_from_slice(&bytes.to_le_bytes());
    }

    fn get_deleted_slot_count(&self) -> u16 {
        u16::from_le_bytes(self[DELETED_SLOT_COUNT_OFFSET..DELETED_SLOT_COUNT_OFFSET + DELETED_SLOT_COUNT_SIZE].try_into().unwrap())
    }

    fn set_deleted_slot_count(&mut self, count: u16) {
        self[DELETED_SLOT_COUNT_OFFSET..DELETED_SLOT_COUNT_OFFSET + DELETED_SLOT_COUNT_SIZE]
            .copy_from_slice(&count.to_le_bytes());
    }

    fn find_next_free_slot(&self) -> usize {
        let start = PAGE_FIXED_HEADER_LEN + HEAP_PAGE_FIXED_METADATA_SIZE;
        let end = self.get_header_size();
        let mut offset = start;
        while offset < end {
            // Length is stored in the last 2 bytes of each slot's metadata
            let length = u16::from_le_bytes(
                self.data[offset + OFFSET_NUM_BYTES..offset + SLOT_METADATA_SIZE]
                    .try_into()
                    .unwrap(),
            );
            if length == 0 {
                return offset;
            }
            offset += SLOT_METADATA_SIZE;
        }
        end // If we are here - all slots are in use, and we should use next one
    }

    fn get_slot_metadata_offset(&self, slot_id: SlotId) -> isize {
        let num_slots = self.get_num_slots() as usize;
        if (slot_id as usize) >= num_slots {
            return -1;
        }
        (PAGE_FIXED_HEADER_LEN + HEAP_PAGE_FIXED_METADATA_SIZE + (slot_id as usize) * SLOT_METADATA_SIZE) as isize
    }

    fn compact_page(&mut self) {
        let num_slots = self.get_num_slots() as usize;
        let header_start = PAGE_FIXED_HEADER_LEN + HEAP_PAGE_FIXED_METADATA_SIZE;

        // 1. Collect all live slot data into Vecs to avoid overlap issues during repacking
        let mut live_slots: Vec<(usize, Vec<u8>)> = Vec::new();
        for i in 0..num_slots {
            let meta_offset = header_start + i * SLOT_METADATA_SIZE;
            let data_offset = u16::from_le_bytes(
                self.data[meta_offset..meta_offset + OFFSET_NUM_BYTES]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let data_length = u16::from_le_bytes(
                self.data[meta_offset + OFFSET_NUM_BYTES..meta_offset + SLOT_METADATA_SIZE]
                    .try_into()
                    .unwrap(),
            ) as usize;
            if data_length > 0 {
                live_slots.push((i, self.data[data_offset..data_offset + data_length].to_vec()));
            }
        }

        // 2. Sort by growing length so largest values end up closest to free_space_ptr,
        //    improving direct reclamation chances on delete.
        live_slots.sort_by_key(|(_, data)| data.len());

        // 3. Repack data contiguously from PAGE_SIZE backward
        let mut new_fsp = PAGE_SIZE;
        for (slot_index, data) in &live_slots {
            new_fsp -= data.len();
            self.data[new_fsp..new_fsp + data.len()].copy_from_slice(data);
            // Update this slot's data offset in its metadata
            let meta_offset = header_start + slot_index * SLOT_METADATA_SIZE;
            self.data[meta_offset..meta_offset + OFFSET_NUM_BYTES]
                .copy_from_slice(&(new_fsp as u16).to_le_bytes());
        }

        // 4. Update free space pointer and reset deleted bytes counter
        self.set_free_space_ptr(new_fsp as u16);
        self.set_deleted_bytes(0);
    }

    ////////////////////////////////////////////////////////////////////////////
    ///                              Main Functions
    ////////////////////////////////////////////////////////////////////////////
    fn init_heap_page(&mut self) {
        //TODO milestone pg
        //Add any initialization code here

        self.set_free_space_ptr(PAGE_SIZE as u16);
    }

    fn add_value(&mut self, bytes: &[u8]) -> Option<SlotId> {
        // 1. Check total free space (gap + reclaimable holes)
        if bytes.len() + SLOT_METADATA_SIZE >= self.get_free_space() {
            return None;
        }

        // 2. Find next free slot
        let slot_offset = self.find_next_free_slot();
        let is_new_slot = slot_offset >= self.get_header_size();

        // 3. Compute how much contiguous gap space we need
        let needed_in_gap = if is_new_slot {
            bytes.len() + SLOT_METADATA_SIZE // data + header growth
        } else {
            bytes.len() // data only, slot metadata already exists
        };

        // 4. If gap alone is too small, compact first (BEFORE touching header)
        let gap = self.get_free_space_ptr() as usize - self.get_header_size();
        if needed_in_gap > gap {
            self.compact_page();
        }

        // 5. Update slot counts
        if is_new_slot {
            self.increment_num_slots();
        } else {
            self.set_deleted_slot_count(self.get_deleted_slot_count() - 1);
        }

        // 6. Compute slot_id
        let slot_id = ((slot_offset - PAGE_FIXED_HEADER_LEN - HEAP_PAGE_FIXED_METADATA_SIZE) / SLOT_METADATA_SIZE) as SlotId;

        // 7. Place the bytes from the free_space_ptr backwards
        let current_fsp = self.get_free_space_ptr() as usize;
        let new_fsp = current_fsp - bytes.len();
        self.data[new_fsp..current_fsp].clone_from_slice(bytes);
        self.set_free_space_ptr(new_fsp as u16);

        // 8. Write the slot metadata (data offset + data length)
        self.data[slot_offset..slot_offset + OFFSET_NUM_BYTES]
            .copy_from_slice(&(new_fsp as u16).to_le_bytes());
        self.data[slot_offset + OFFSET_NUM_BYTES..slot_offset + SLOT_METADATA_SIZE]
            .copy_from_slice(&(bytes.len() as u16).to_le_bytes());

        Some(slot_id)
    }

    fn get_value(&self, slot_id: SlotId) -> Option<&[u8]> {
        // 1. Find offset of slot metadata (returns -1 if slot_id is out of range)
        let offset = self.get_slot_metadata_offset(slot_id);
        if offset < 0 {
            return None;
        }
        let slot_metadata_offset = offset as usize;

        // Parse the slot metadata (offset and length)
        let data_offset = u16::from_le_bytes(
            self.data[slot_metadata_offset..slot_metadata_offset + OFFSET_NUM_BYTES]
                .try_into()
                .unwrap()
        ) as usize;

        let data_length = u16::from_le_bytes(
            self.data[slot_metadata_offset + OFFSET_NUM_BYTES..slot_metadata_offset + SLOT_METADATA_SIZE]
                .try_into()
                .unwrap()
        ) as usize;

        // 3. Check if the slot is deleted (length == 0 means deleted slot)
        if data_length == 0 {
            return None;
        }

        // Get the offset and length to read appropriate bytes and return them
        Some(&self.data[data_offset..data_offset + data_length])
    }

    fn delete_value(&mut self, slot_id: SlotId) -> Option<()> {
        // 1. Find offset of slot metadata
        let offset = self.get_slot_metadata_offset(slot_id);
        if offset < 0 {
            return None;
        }
        let slot_metadata_offset = offset as usize;

        // 2. Read data offset and length from slot metadata
        let data_offset = u16::from_le_bytes(
            self.data[slot_metadata_offset..slot_metadata_offset + OFFSET_NUM_BYTES]
                .try_into()
                .unwrap()
        ) as usize;

        let data_length: usize = u16::from_le_bytes(
            self.data[slot_metadata_offset + OFFSET_NUM_BYTES..slot_metadata_offset + SLOT_METADATA_SIZE]
                .try_into()
                .unwrap()
        ) as usize;

        // Already deleted
        if data_length == 0 {
            return None;
        }

        // 3. Zero out the data bytes
        for i in data_offset..data_offset + data_length {
            self.data[i] = 0;
        }

        // 4. Set the length in slot metadata to 0
        self.data[slot_metadata_offset + OFFSET_NUM_BYTES..slot_metadata_offset + SLOT_METADATA_SIZE]
            .copy_from_slice(&0u16.to_le_bytes());

        // 5. Reclaim space: if the deleted data is at the free_space_ptr boundary,
        //    advance the pointer directly. Otherwise, track it as a hole.
        if data_offset == self.get_free_space_ptr() as usize {
            self.set_free_space_ptr((data_offset + data_length) as u16);
        } else {
            self.set_deleted_bytes(self.get_deleted_bytes() + data_length as u16);
        }
        self.set_deleted_slot_count(self.get_deleted_slot_count() + 1);

        Some(())
    }

    fn update_value(&mut self, slot_id: SlotId, bytes: &[u8]) -> Option<()> {
        panic!("TODO milestone pg");
    }

    #[allow(dead_code)]
    fn get_header_size(&self) -> usize {
        // Header is the fixed header + the slot metadata
        // 
        // To count number of bytes used by the slots, we calculate the number of
        // Slots that are currently in use and deleted (inner fragmentation), and
        // multiply that by the size of the SLOT_METADATA_SIZE
        PAGE_FIXED_HEADER_LEN + HEAP_PAGE_FIXED_METADATA_SIZE + self.get_num_slots() as usize * SLOT_METADATA_SIZE
    }

    #[allow(dead_code)]
    fn get_free_space(&self) -> usize {
        (self.get_free_space_ptr() as usize - self.get_header_size()) + self.get_deleted_bytes() as usize
    }

    fn iter(&self) -> HeapPageIter<'_> {
        HeapPageIter {
            page: self,
            //TODO milestone pg
            //Initialize with added variables here
        }
    }
}

pub struct HeapPageIter<'a> {
    page: &'a Page,
    //TODO milestone pg
    // Add any variables here
}

impl<'a> Iterator for HeapPageIter<'a> {
    type Item = (&'a [u8], SlotId);

    /// This function will return the next value in the page. It should return
    /// None if there are no more values in the page.
    /// The iterator should return the bytes reference and the slotId for each value in the page as a tuple.
    fn next(&mut self) -> Option<Self::Item> {
        panic!("TODO milestone pg");
    }
}

/// The implementation of IntoIterator which allows an iterator to be created
/// for a page. This should create the PageIter struct with the appropriate state/metadata
/// on initialization.
impl<'a> IntoIterator for &'a Page {
    type Item = (&'a [u8], SlotId);
    type IntoIter = HeapPageIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        HeapPageIter {
            page: self,
            //TODO milestone pg
            //Initialize with added variables here
        }
    }
}
