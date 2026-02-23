# Write up

Sasha Timokhin

## Design

### HeapFile
I designed heap file just as a sequence of heap pages. To get the index of a page, I just use `offset (index) * PAGE_SIZE`. That way, when I try to locate page I do not need to search for anything in the header of the file.

In this current implementation header is a dummy. Also, when I decide where to store the slot (in which page), I do not use any smart system. I do not use the header space to keep track of the space used in all of the pages, though I could to optimize the find time for the correct location of the slot.

I decided not to implement any kind of tracking currently, but I might consider doing that in the future (honestly, if I do not get a great scroe on the benchmarking test, I will implement some smarter system to find new appropriate location).

### StorageManager

StorageManager is just a thin wrapper around HeapFile. All operations look up the correct `Arc<HeapFile>` from the `cid_heapfile_map` and delegate to the corresponding HeapFile method directly.

One important decision was to use `HeapFile::load()` instead of `HeapFile::new()` when reloading existing containers on startup. Using `new()` would allocate a second header page at the end of the container, leaving a wasted page with no valid data.

For `update_val`, I had to handle the case where the new value is larger than the old one and does not fit on the same page. In that case, instead of returning an error, I delete the old value and re-insert the new one via `add_val`, which may place it on a different page and return a different `ValueId`. The caller is responsible for tracking the updated `ValueId`. This was needed to make the variable-size update benchmark work correctly.

I also added page bounds checking in `get_val`, `delete_val`, and `update_val` before passing the request down to the buffer pool. Without this, accessing a non-existent page hits a `debug_assert` in `BaseFile` and panics instead of returning a clean error.

### HeapFileIter
Nothing too interesting - I just added a new variable for keeping track of the current page we are iterating through. 

## Time Estimate / Reflection 

This assignment took me about 2 days to complete. Most of the time was spent exploring the code, and making sure that I understand what I needed to implement.

## Incomplete

N/A

## References

* https://doc.rust-lang.org/std/sync/struct.RwLock.html
* https://doc.rust-lang.org/std/sync/struct.Arc.html
* https://doc.rust-lang.org/std/iter/trait.Iterator.html
* https://doc.rust-lang.org/std/mem/fn.transmute.html
* https://docs.rs/libc/latest/libc/
* https://man7.org/linux/man-pages/man2/pread.2.html