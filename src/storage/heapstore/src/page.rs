pub use crate::heap_page::HeapPage;
use crate::heap_page::HEAP_PAGE_FIXED_METADATA_SIZE;
use common::ids::CheckSum;
use common::prelude::*;
use common::PAGE_SIZE;
use std::fmt;
use std::fmt::Write;
use std::hash::Hasher;
use std::mem;
use std::ops::Deref;
use std::ops::DerefMut;

/// Data type to hold any value smaller than the size of a page.
/// We choose u16 because it is sufficient to represent any slot that fits in a 4096-byte-sized page.
/// Note that you will need to cast Offset to usize if you want to use it to index an array.
pub type Offset = u16;

/// How many bytes are in an offset.
#[allow(dead_code)]
pub const OFFSET_NUM_BYTES: usize = mem::size_of::<Offset>();

/// For debugging purposes only
const BYTES_PER_LINE: usize = 40;

#[allow(dead_code)]
pub const PAGE_ID_SIZE: usize = mem::size_of::<PageId>();
#[allow(dead_code)]
pub const SLOT_ID_SIZE: usize = mem::size_of::<SlotId>();
#[allow(dead_code)]
pub const CHECKSUM_SIZE: usize = mem::size_of::<CheckSum>();

#[allow(dead_code)]
pub const LSN_PAGE_OFFSET: usize = PAGE_ID_SIZE;
#[allow(dead_code)]
pub const LSN_SLOT_OFFSET: usize = LSN_PAGE_OFFSET + PAGE_ID_SIZE;
#[allow(dead_code)]
pub const CHECKSUM_OFFSET: usize = LSN_SLOT_OFFSET + SLOT_ID_SIZE;

/// The number of bytes reserved for the fixed header of all pages.
pub const PAGE_FIXED_HEADER_LEN: usize = 16;

// PG MS Add any additional header fields or constants/metadata you need here.

#[cfg(test)]
const _: () = {
    /// Verify that the fixed header is the large enough to hold the page id, lsn, and checksum.
    use common::ids::CheckSum;
    pub const LSN_SIZE: usize = mem::size_of::<Lsn>();
    pub const CHECKSUM_SIZE: usize = mem::size_of::<CheckSum>();
    assert!(PAGE_FIXED_HEADER_LEN >= (PAGE_ID_SIZE + LSN_SIZE + CHECKSUM_SIZE));
};

/// Page struct. This must always occupy `PAGE_SIZE`` bytes at all times.
/// In the header, 16 bytes (`PAGE_FIXED_HEADER_LEN`) are reserved for general page metadata.
/// The first bytes are determined by the the size of the PageId, Lsn, and the CheckSum (details below).
/// You are allowed the remaining bytes for metadata or any optimization.
/// So if the PageId is 4 bytes, Lsn is 8 bytes, and CheckSum is 2 bytes your "used" header size
/// would be 4+8+2=14 bytes. Leaving you 16-14=2 bytes for your own use (or not) Note that an implementation
/// of the Page may have it's own metadata header that will be placed after the first `PAGE_FIXED_HEADER_LEN` bytes.
pub struct Page {
    /// For holding data/bytes. No other fields are allowed in this struct.
    pub(crate) data: [u8; PAGE_SIZE],
}

/// The functions required for page
impl Page {
    /// Create a new page
    /// The default LSN and CheckSum are set to 0.
    /// HINT: To convert a variable x to bytes using little endian, use
    /// x.to_le_bytes()
    pub fn new(page_id: PageId) -> Self {

        // Note - though by definition we are passing page_id to be of the size PrageId,
        // we are still checking that the size is appropriate in case the implementation
        // changes at any point in the future
        assert!(common::fits_in_type::<[u8; PAGE_ID_SIZE], _>(&page_id));

        // 1 - PageId (4 bytes)
        let page_id_bytes: [u8; _] = page_id.to_le_bytes();
        let mut data: [u8; 4096] = [0u8; PAGE_SIZE];
        data[..PAGE_ID_SIZE].copy_from_slice(&page_id_bytes);

        // 2 - LSN is two components: PageId (4 bytes) + SlotId (2 bytes), both default to 0
        let lsn_page: PageId = 0;
        let lsn_slot: SlotId = 0;
        data[LSN_PAGE_OFFSET..LSN_PAGE_OFFSET + PAGE_ID_SIZE].copy_from_slice(&lsn_page.to_le_bytes());
        data[LSN_SLOT_OFFSET..LSN_SLOT_OFFSET + SLOT_ID_SIZE].copy_from_slice(&lsn_slot.to_le_bytes());

        // 3 - Checksum (2 bytes). Defaults to 0
        let checksum: CheckSum = 0;
        data[CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_SIZE].copy_from_slice(&checksum.to_le_bytes());

        // 4 - Spare bytes to fill the fixed header to PAGE_FIXED_HEADER_LEN (16 bytes)
        let spare_offset = CHECKSUM_OFFSET + CHECKSUM_SIZE;
        let spare_bytes = [0u8; PAGE_FIXED_HEADER_LEN - (CHECKSUM_OFFSET + CHECKSUM_SIZE)];
        data[spare_offset..PAGE_FIXED_HEADER_LEN].copy_from_slice(&spare_bytes);

        // 5 - Heap page metadata (8 bytes)
        let heap_meta = [0u8; HEAP_PAGE_FIXED_METADATA_SIZE];
        data[PAGE_FIXED_HEADER_LEN..PAGE_FIXED_HEADER_LEN + HEAP_PAGE_FIXED_METADATA_SIZE]
            .copy_from_slice(&heap_meta);

        Page { data }
    }

    /// Create a new empty page
    pub fn new_empty() -> Self {
        panic!("TODO milestone pg");
    }

    /// Return the page id for a page
    ///
    /// HINT to create a primitive data type from a slice you can use the following
    /// (the example is for a u16 type and the data store in little endian)
    /// u16::from_le_bytes(data[X..Y].try_into().unwrap());
    ///
    /// Arguments:
    /// * `&self` - a reference to the page
    ///
    /// Returns:
    /// * `PageId` - the page id
    pub fn get_page_id(&self) -> PageId {
        PageId::from_le_bytes(self.data[0..PAGE_ID_SIZE].try_into().unwrap())
    }

    /// Set the page id for a page
    /// Hint: to convert a primitive data type to bytes you can use the following
    /// x.to_le_bytes()
    ///
    /// Hint: To overwrite a slice of bytes you can use copy_from_slice function
    /// that takes the slice (part) of the array you want to overwrite and the reference to the
    /// bytes you want to copy. https://doc.rust-lang.org/std/primitive.slice.html#method.copy_from_slice
    ///
    /// Arguments:
    /// * `&self` - a mutable reference to the page
    /// * `page_id` - the page id to set
    pub fn set_page_id(&mut self, page_id: PageId) {
        self.data[0..PAGE_ID_SIZE].copy_from_slice(&page_id.to_le_bytes());
    }

    /// Get the LSN for the page. The LSN is a log sequence number that is used to
    /// identify the last update to the page. It is used for recovery and logging purposes.
    /// The LSN is a combination of the page id and the slot id.
    /// The LSN is stored in the page header after the page id.
    ///
    /// Arguments:
    /// * `&self` - a reference to the page
    ///
    /// Returns:
    /// * `Lsn` - the LSN for the page
    pub fn get_lsn(&self) -> Lsn {
        let page_id = PageId::from_le_bytes(
            self.data[LSN_PAGE_OFFSET..LSN_PAGE_OFFSET + PAGE_ID_SIZE].try_into().unwrap(),
        );
        let slot_id = SlotId::from_le_bytes(
            self.data[LSN_SLOT_OFFSET..LSN_SLOT_OFFSET + SLOT_ID_SIZE].try_into().unwrap(),
        );
        Lsn::new(page_id, slot_id)
    }

    /// Set the LSN for the page. The LSN is a log sequence number that is used to
    /// identify the last update to the page. It is used for recovery and logging purposes.
    /// The LSN is a combination of the page id and the slot id.
    /// The LSN is stored in the page header after the page id.
    ///
    /// NOTE: This should only set an LSN if it is greater than the current LSN.
    ///
    /// Arguments:
    /// * `&self` - a mutable reference to the page
    /// * `lsn` - the LSN to set
    pub fn set_lsn(&mut self, lsn: Lsn) {
        let current = self.get_lsn();
        if lsn > current {
            self.data[LSN_PAGE_OFFSET..LSN_PAGE_OFFSET + PAGE_ID_SIZE]
                .copy_from_slice(&lsn.page_id.to_le_bytes());
            self.data[LSN_SLOT_OFFSET..LSN_SLOT_OFFSET + SLOT_ID_SIZE]
                .copy_from_slice(&lsn.slot_id.to_le_bytes());
        }
    }

    /// Get the checksum for the page. The checksum is used to verify the integrity of the page.
    /// The checksum is stored in the page header after the LSN.
    ///     
    /// Arguments:
    /// * `&self` - a reference to the page
    ///
    /// Returns:
    /// * `CheckSum` - the checksum for the page
    pub fn get_checksum(&self) -> CheckSum {
        CheckSum::from_le_bytes(
            self.data[CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_SIZE].try_into().unwrap(),
        )
    }

    /// Set the checksum for the page. The checksum is used to verify the integrity of the page.
    /// The checksum is a hash function of the full page bytes except the CRC of the page which
    /// should be zero'd out for the checksum calculation.
    ///
    /// Arguments:
    /// * `&self` - a mutable reference to the page
    pub fn set_checksum(&mut self) {
        // Zero out the checksum field before computing
        self.data[CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_SIZE].copy_from_slice(&[0u8; CHECKSUM_SIZE]);

        // Hash the full page bytes
        let mut hasher = std::hash::DefaultHasher::new();
        hasher.write(&self.data[..]);
        let checksum_64 = hasher.finish();
        let checksum = (checksum_64 & 0xFFFF) as u16;

        // Write the checksum back
        self.data[CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_SIZE]
            .copy_from_slice(&checksum.to_le_bytes());
    }

    /// Create a page from a byte array
    pub fn from_bytes(data: [u8; PAGE_SIZE]) -> Self {
        Page { data }
    }

    /// Get a reference to the bytes of the page
    pub fn to_bytes(&self) -> &[u8; PAGE_SIZE] {
        &self.data
    }

    /// Get a mutable reference to the bytes of the page
    pub fn to_bytes_mut(&mut self) -> &mut [u8; PAGE_SIZE] {
        &mut self.data
    }

    /// Utility function for comparing the bytes of another page.
    /// Returns a vec of Offset and byte diff
    #[allow(dead_code)]
    pub fn compare_page(&self, other_page: Vec<u8>) -> Vec<(Offset, Vec<u8>)> {
        let mut res = Vec::new();
        let bytes = self.to_bytes();
        assert_eq!(bytes.len(), other_page.len());
        let mut in_diff = false;
        let mut diff_start = 0;
        let mut diff_vec: Vec<u8> = Vec::new();
        for (i, (b1, b2)) in bytes.iter().zip(&other_page).enumerate() {
            if b1 != b2 {
                if !in_diff {
                    diff_start = i;
                    in_diff = true;
                }
                diff_vec.push(*b1);
            } else if in_diff {
                //end the diff
                res.push((diff_start as Offset, diff_vec.clone()));
                diff_vec.clear();
                in_diff = false;
            }
        }
        res
    }
}

/// The implementation to create a clone of a page
impl Clone for Page {
    fn clone(&self) -> Self {
        Page { data: self.data }
    }
}

impl Deref for Page {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.data[PAGE_FIXED_HEADER_LEN..]
    }
}

impl DerefMut for Page {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data[PAGE_FIXED_HEADER_LEN..]
    }
}

/// A custom implementation of the Debug trait for the Page struct.
/// This implementation formats the page data in a human-readable way.
/// Use the {:?} format specifier to print the page debug data.
impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        //let bytes: &[u8] = unsafe { any_as_u8_slice(&self) };
        let p = self.to_bytes();
        let mut buffer = String::new();
        let len_bytes = p.len();

        writeln!(
            &mut buffer,
            "PID:{} LSN:{} Checksum:{}",
            self.get_page_id(),
            self.get_lsn(),
            self.get_checksum()
        )
        .unwrap();
        let mut pos = 0;
        let mut remaining;
        let mut empty_lines_count = 0;
        let comp = [0; BYTES_PER_LINE];
        //hide the empty lines
        while pos < len_bytes {
            remaining = len_bytes - pos;
            if remaining > BYTES_PER_LINE {
                let pv = &(p)[pos..pos + BYTES_PER_LINE];
                if pv.eq(&comp) {
                    empty_lines_count += 1;
                    pos += BYTES_PER_LINE;
                    continue;
                }
                if empty_lines_count != 0 {
                    write!(&mut buffer, "{} ", empty_lines_count).unwrap();
                    buffer += "empty lines were hidden\n";
                    empty_lines_count = 0;
                }
                // for hex offset
                write!(&mut buffer, "[{:4}] ", pos).unwrap();
                #[allow(clippy::needless_range_loop)]
                for i in 0..BYTES_PER_LINE {
                    match pv[i] {
                        0x00 => buffer += ".  ",
                        0xff => buffer += "## ",
                        _ => write!(&mut buffer, "{:02x} ", pv[i]).unwrap(),
                    };
                }
            } else {
                let pv = &(*p)[pos..pos + remaining];
                if pv.eq(&comp) {
                    empty_lines_count += 1;
                    pos += BYTES_PER_LINE;
                    continue;
                }
                if empty_lines_count != 0 {
                    write!(&mut buffer, "{} ", empty_lines_count).unwrap();
                    buffer += "empty lines were hidden\n";
                    empty_lines_count = 0;
                }
                // for hex offset
                //buffer += &format!("[0x{:08x}] ", pos);
                write!(&mut buffer, "[{:4}] ", pos).unwrap();
                #[allow(clippy::needless_range_loop)]
                for i in 0..remaining {
                    match pv[i] {
                        0x00 => buffer += ".  ",
                        0xff => buffer += "## ",
                        _ => write!(&mut buffer, "{:02x} ", pv[i]).unwrap(),
                    };
                }
            }
            buffer += "\n";
            pos += BYTES_PER_LINE;
        }
        if empty_lines_count != 0 {
            write!(&mut buffer, "{} ", empty_lines_count).unwrap();
            buffer += "empty lines were hidden\n";
        }
        write!(f, "{}", buffer)
    }
}
