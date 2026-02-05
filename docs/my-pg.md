# Write up

Sasha Timokhin

## Design

Pages follow the slotted-page design to implement the variable length slots.
Page consists of two parts: Header and Body, both of variable length.

Header grows from low addresses to big addresses
Body grows from high addresses to low addresses.

Header is further subdivided into FIXED header (includes pageID, LSN, Checksum, Heap Metadata)
and slot metadatas.

Notice: order of the slot metadatas is important - offset of the slots meatadata
corresponds to the id of the slot, relative to the page. For example, if the offset
of the slot meatadata is 4 bytes from where the FIXED header ends, the id of that
slot is 1, since the length of the metadata block is 4 bytes.

### Heap Metadata Structure
I used 4 variables to optimize different operations within the page.
1) `num_slots`: It counts the number of the slots that are currently active and deleted. The point of this variable is to set the number of slot metadatas used, to optimize finding the header size and the free space of the page. It uses 2 bytes, because in total there are `4096 - 24 = 4072 bytes` that can be used by the slots, which equates to `1012 slot metadatas`, which need 10 bits, or rounded 2 bytes.
2) `free_space_ptr`: a pointer that points to the next free space in the body space. It helps to decide how much free space there is in the page (in combination with `num_slots`, `deleted_bytes`). Like other pointers in the page, this one takes up 2 bytes.
3) `deleted_bytes` - count of bytes that are freed after allocation (number of bytes in the internal fragmentation section of the body, if delete_value is called). Helps to keep track of the number of free bytes in the page. Takes up 2 bytes.
4) `deleted_slots` - helps to keep track to the number of slots that are not allocated anymore. That helps to track the number of free bytes in the slot metadata space, to count the free space. Takes up 2 bytes.

### Reallocation of space in the body
Since we support a deletion operation, pages are subject to possible internal fragementation. To reuse the space in the body segment of the page, I decided to use compaction technique:
```text
Step 1: Through the use of num_slots, free_space_ptr, deleted_bytes, and deleted_slots we calculate how much space there is free in the page
Step 2: If there is enough space to put in new slot (considering reusing a slot metadata if appropriate!), we check if we can put that data at free_space_ptr continiously.
Step 3a: If we can put it there, we just do that
Step 3b: If putting the data in that place is impossible, we will compact data by retrieving all data blocks that are set active, copying them into a separate array, ordering them so that the smaller offsets have larger slots. (That is needed so that if we delete the largest value we can simply move free_space_ptr without introducing inner fragmentation). And setting the free_space_ptr to point to the end of the sorted array of values.
```

The reason why I do not consider filling in the inner fragments if they big enough before trying to doing compaction, is of the efficiency. To check if we have enough space to fit in the new value requires to iterate over all of the slots to find the empty value slot. Time complexity is `O(n)`, not good.

Then if we find the spot, insertion is `O(1)`, but if we do not, we will need to do a compaction anyway. Time complexity of compaction of a page is `O(n * k)`, where `n` - number of slots, and `k` - is a number of bytes set.

If I attempt to do Space Reclamation like in the example, in the best case time complexity is `O(n)`, and worst case is still `O(n)` (complexity of compaction operation). Hence, space reclamation from the example is wasteful.

Besides, the maximum number of slots in a page is ~1000. I do not think that we should concern ourselves with further optimizations right now, unless the scale increases signifcanlty.

## Time Estimate / Reflection 

Cummutivelly, it took me 6 hours to complete both designing and developing part of the homework. In the past, I took CMSC 23000 (Operating Systems), and I had to work with pages, low-level pointers before. Conceptually, designing was not too difficult. Most of my time was spent reviewing Rust documentation. I do not feel comfortable to write in Rust thus far, as it is my first time I use it.

## Incomplete

All is complete!

## References

* https://doc.rust-lang.org/stable/
* https://web.mit.edu/rust-lang_v1.25/arch/amd64_ubuntu1404/share/doc/rust/html/book/first-edition/documentation.html
* https://roadmap.sh/rust
* https://boxoflearn.com/what-are-the-functions-in-rust/
* https://www.youtube.com/watch?v=nOKOFYzvvHo&t=97s