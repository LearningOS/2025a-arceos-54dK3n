//! Allocator algorithm in lab.

#![no_std]

use allocator::{AllocError, AllocResult, BaseAllocator, ByteAllocator};
use core::alloc::Layout;
use core::mem::{align_of, size_of};
use core::ptr::{self, NonNull};

#[repr(C)]
struct FreeNode {
    size: usize,
    next: *mut FreeNode,
}

#[repr(C)]
struct AllocHeader {
    block_start: usize,
    block_size: usize,
}

const NODE_ALIGN: usize = align_of::<FreeNode>();
const MIN_FREE_BLOCK_SIZE: usize = size_of::<FreeNode>();

pub struct LabByteAllocator {
    head: *mut FreeNode,
    total_bytes: usize,
    used_bytes: usize,
}

// Safety: all interior mutation is protected by the outer allocator lock.
unsafe impl Send for LabByteAllocator {}

impl LabByteAllocator {
    pub const fn new() -> Self {
        Self {
            head: ptr::null_mut(),
            total_bytes: 0,
            used_bytes: 0,
        }
    }

    #[inline]
    const fn align_up(addr: usize, align: usize) -> usize {
        (addr + align - 1) & !(align - 1)
    }

    fn normalize_region(start: usize, size: usize) -> Option<(usize, usize)> {
        let end = start.checked_add(size)?;
        let aligned_start = Self::align_up(start, NODE_ALIGN);
        let aligned_end = end & !(NODE_ALIGN - 1);
        if aligned_end <= aligned_start {
            return None;
        }
        let aligned_size = aligned_end - aligned_start;
        if aligned_size < MIN_FREE_BLOCK_SIZE {
            None
        } else {
            Some((aligned_start, aligned_size))
        }
    }

    unsafe fn write_free_node(&mut self, start: usize, size: usize, next: *mut FreeNode) {
        let node = start as *mut FreeNode;
        node.write(FreeNode { size, next });
    }

    unsafe fn insert_free_block(&mut self, start: usize, size: usize) -> AllocResult {
        let mut prev: *mut FreeNode = ptr::null_mut();
        let mut curr = self.head;

        while !curr.is_null() && (curr as usize) < start {
            prev = curr;
            curr = (*curr).next;
        }

        let mut merged_start = start;
        let mut merged_size = size;

        if !prev.is_null() {
            let prev_end = prev as usize + (*prev).size;
            if start < prev_end {
                return Err(AllocError::MemoryOverlap);
            }
            if start == prev_end {
                merged_start = prev as usize;
                merged_size += (*prev).size;
                prev = Self::find_prev(self.head, prev);
            }
        }

        while !curr.is_null() {
            let curr_start = curr as usize;
            if merged_start + merged_size < curr_start {
                break;
            }
            if merged_start + merged_size == curr_start {
                merged_size += (*curr).size;
                curr = (*curr).next;
            } else {
                return Err(AllocError::MemoryOverlap);
            }
        }

        let new_next = curr;
        if prev.is_null() {
            self.write_free_node(merged_start, merged_size, new_next);
            self.head = merged_start as *mut FreeNode;
        } else {
            self.write_free_node(merged_start, merged_size, new_next);
            (*prev).next = merged_start as *mut FreeNode;
        }
        Ok(())
    }

    unsafe fn find_prev(mut head: *mut FreeNode, target: *mut FreeNode) -> *mut FreeNode {
        let mut prev = ptr::null_mut();
        while !head.is_null() && head != target {
            prev = head;
            head = (*head).next;
        }
        prev
    }
}

impl BaseAllocator for LabByteAllocator {
    fn init(&mut self, start: usize, size: usize) {
        self.head = ptr::null_mut();
        self.total_bytes = 0;
        self.used_bytes = 0;
        self.add_memory(start, size)
            .expect("invalid initial heap region");
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        let Some((start, size)) = Self::normalize_region(start, size) else {
            return Err(AllocError::InvalidParam);
        };
        unsafe {
            self.insert_free_block(start, size)?;
        }
        self.total_bytes += size;
        Ok(())
    }
}

impl ByteAllocator for LabByteAllocator {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        if layout.size() == 0 {
            return Ok(NonNull::dangling());
        }

        let header_size = size_of::<AllocHeader>();
        let header_align = align_of::<AllocHeader>();
        let min_align = layout.align().max(header_align);

        let mut prev: *mut FreeNode = ptr::null_mut();
        let mut curr = self.head;

        while !curr.is_null() {
            let block_start = curr as usize;
            let block_size = unsafe { (*curr).size };
            let next = unsafe { (*curr).next };

            let mut user_start = Self::align_up(block_start + header_size, min_align);
            let mut prefix_size = user_start - header_size - block_start;
            if prefix_size != 0 && prefix_size < MIN_FREE_BLOCK_SIZE {
                user_start =
                    Self::align_up(block_start + MIN_FREE_BLOCK_SIZE + header_size, min_align);
                prefix_size = user_start - header_size - block_start;
            }

            let Some(raw_end) = user_start.checked_add(layout.size()) else {
                return Err(AllocError::NoMemory);
            };
            let mut consumed = Self::align_up(raw_end - block_start, NODE_ALIGN);
            if consumed > block_size {
                prev = curr;
                curr = next;
                continue;
            }

            let mut suffix_size = block_size - consumed;
            if suffix_size != 0 && suffix_size < MIN_FREE_BLOCK_SIZE {
                consumed = block_size;
                suffix_size = 0;
            }

            if prev.is_null() {
                self.head = next;
            } else {
                unsafe {
                    (*prev).next = next;
                }
            }

            if prefix_size != 0 {
                unsafe {
                    self.insert_free_block(block_start, prefix_size)
                        .expect("allocator free-list corrupted");
                }
            }
            if suffix_size != 0 {
                unsafe {
                    self.insert_free_block(block_start + consumed, suffix_size)
                        .expect("allocator free-list corrupted");
                }
            }

            let header_ptr = (user_start - header_size) as *mut AllocHeader;
            unsafe {
                header_ptr.write(AllocHeader {
                    block_start,
                    block_size: consumed,
                });
            }
            self.used_bytes += layout.size();
            return NonNull::new(user_start as *mut u8).ok_or(AllocError::NoMemory);
        }

        Err(AllocError::NoMemory)
    }

    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return;
        }

        let header_ptr = (pos.as_ptr() as usize - size_of::<AllocHeader>()) as *const AllocHeader;
        let header = unsafe { header_ptr.read() };
        self.used_bytes -= layout.size();
        unsafe {
            self.insert_free_block(header.block_start, header.block_size)
                .expect("double free or corrupted allocation header");
        }
    }

    fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    fn available_bytes(&self) -> usize {
        self.total_bytes - self.used_bytes
    }
}
