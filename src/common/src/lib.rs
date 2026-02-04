extern crate csv;
#[macro_use]
extern crate serde;
#[macro_use]
extern crate log;

pub mod error;
pub mod ids;
pub mod util;
pub use util::common_test_util as testutil;

/// Page size in bytes
pub const PAGE_SIZE: usize = 4096;

// How many pages a buffer pool can hold
pub const PAGE_SLOTS: usize = 50;
// Maximum number of columns in a table
pub const MAX_COLUMNS: usize = 100;

// dir name of manager table
pub const MANAGERS_DIR_NAME: &str = "managers";

// dir name of manager table
pub const QUERY_CACHES_DIR_NAME: &str = "query_caches";

pub mod prelude {
    pub use crate::error::CrustyError;
    pub use crate::ids::Permissions;
    pub use crate::ids::{
        ColumnId, ContainerId, LogicalTimeStamp, Lsn, PageId, SlotId, StateType, TidType,
        TransactionId, ValueId,
    };
}

pub use crate::error::{ConversionError, CrustyError};

/// Checks if the in-memory size of a value's type `V` fits within the size of the target type `T`.
///
/// Returns `true` if `size_of::<V>() <= size_of::<T>()`, `false` otherwise.
///
/// # Examples
/// ```
/// use common::fits_in_type;
/// // u16 (2 bytes) fits within u32 (4 bytes)
/// let val: u16 = 42;
/// assert!(fits_in_type::<u32, _>(&val));
/// ```
pub fn fits_in_type<T, V>(_value: &V) -> bool {
    std::mem::size_of::<V>() <= std::mem::size_of::<T>()
}
