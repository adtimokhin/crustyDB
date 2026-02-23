use crate::file_stats::FileStats;
use crate::page::Page;
use common::ids::{ContainerId, PageId};
use common::PAGE_SIZE;
use libc::{c_void, fsync, pread, pwrite};
use std::fs::{File, OpenOptions};
use std::mem::MaybeUninit;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

/// BaseFileTrait is a trait that defines the methods that are required to manage the file that is used to store the pages.
pub trait BaseFileTrait: Send + Sync {
    fn num_pages(&self) -> usize;
    fn get_stats(&self) -> FileStats;
    #[allow(dead_code)]
    fn prefetch_page(&self, page_id: PageId) -> Result<(), std::io::Error>;
    /// Read a page from the file at the given page_id and store it in the given page.
    fn read_page(&self, page_id: PageId, page: &mut Page) -> Result<(), std::io::Error>;
    /// Write a page to the file at the given page_id.
    fn write_page(&self, page_id: PageId, page: &Page) -> Result<(), std::io::Error>;
    /// Flush the file to disk if necessary.
    fn flush(&self) -> Result<(), std::io::Error>;
}

/// BaseFile is a structure that is used to manage the file that is used to store the pages.
/// It is responsible for reading and writing pages to the file.
#[allow(dead_code)]
pub struct BaseFile {
    _path: PathBuf,
    _file: File, // When this file is dropped, the file descriptor (file_no) will be invalid.
    stats: FileStats,
    file_no: i32,
    direct: bool,
}

impl BaseFile {
    pub fn new<P: AsRef<std::path::Path>>(
        db_dir: P,
        c_id: ContainerId,
    ) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&db_dir)?;
        let path = db_dir.as_ref().join(format!("{}", c_id));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let file_no = file.as_raw_fd();
        Ok(BaseFile {
            _path: path,
            _file: file,
            stats: FileStats::new(),
            file_no,
            direct: true,
        })
    }
}

impl BaseFileTrait for BaseFile {
    fn num_pages(&self) -> usize {
        // Allocate uninitialized memory for libc::stat
        let mut stat = MaybeUninit::<libc::stat>::uninit();

        // Call fstat with a pointer to our uninitialized stat buffer
        let ret = unsafe { libc::fstat(self.file_no, stat.as_mut_ptr()) };

        // Check for errors (fstat returns -1 on failure)
        if ret == -1 {
            return 0;
        }

        // Now that fstat has successfully written to the buffer,
        // we can assume it is initialized.
        let stat = unsafe { stat.assume_init() };

        // Use the file size (st_size) from stat, then compute pages.
        (stat.st_size as usize) / PAGE_SIZE
    }

    fn get_stats(&self) -> FileStats {
        self.stats.clone()
    }

    fn prefetch_page(&self, _page_id: PageId) -> Result<(), std::io::Error> {
        Ok(())
    }

    /// Read a page from the file at the given page_id and store it in the given page.
    fn read_page(&self, page_id: PageId, page: &mut Page) -> Result<(), std::io::Error> {
        debug_assert!(
            page_id < self.num_pages() as u32,
            "Page id out of bounds. Try to read page_id {} from file with {} pages",
            page_id,
            self.num_pages()
        );
        self.stats.inc_read_count(self.direct);
        unsafe {
            // Use pread to read the page from the file.
            // you will need to use the file descriptor (file_no) to read the page,
            // the page as write buffer, the amount of bytes to read,
            // and the offset to read from. You will want to check the
            // return value of pread to make sure that the read was successful and
            // the expected amount of data was read. If not you should:
            // return Err(std::io::Error::last_os_error());
            //
            // HINT to cast the page to a mutable pointer use:
            // page.to_bytes_mut().as_mut_ptr() as *mut c_void

            panic!("TODO milestone hs");
        }
        debug_assert!(page.get_page_id() == page_id, "Page id mismatch");
        Ok(())
    }

    /// Write a page to the file at the given page_id.
    fn write_page(&self, page_id: PageId, page: &Page) -> Result<(), std::io::Error> {
        self.stats.inc_write_count(self.direct);
        debug_assert!(page.get_page_id() == page_id, "Page id mismatch");
        unsafe {
            // Use pwrite to write the page to the file.
            // you will need to use the file descriptor (file_no) to write the page,
            // the page as read buffer, the amount of bytes to write,
            // and the offset to write to. You will want to check the
            // return value of pwrite to make sure that the write was successful and
            // the expected amount of data was written. If not you should:
            // return Err(std::io::Error::last_os_error());
            //
            // HINT to cast the page to a pointer use:
            // page.to_bytes().as_ptr() as *const c_void
            panic!("TODO milestone hs");
        }
        Ok(())
    }

    // With psync_direct, we don't need to flush.
    fn flush(&self) -> Result<(), std::io::Error> {
        if self.direct {
            Ok(())
        } else {
            unsafe {
                let ret = fsync(self.file_no);
                if ret != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }
}
