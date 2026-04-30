#![no_std]

use allocator::{BaseAllocator, ByteAllocator, PageAllocator};

const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

const fn align_down(addr: usize, align: usize) -> usize {
    addr & !(align - 1)
}

/// Early memory allocator
/// Use it before formal bytes-allocator and pages-allocator can work!
/// This is a double-end memory range:
/// - Alloc bytes forward
/// - Alloc pages backward
///
/// [ bytes-used | avail-area | pages-used ]
/// |            | -->    <-- |            |
/// start       b_pos        p_pos       end
///
/// For bytes area, 'count' records number of allocations.
/// When it goes down to ZERO, free bytes-used area.
/// For pages area, it will never be freed!
///
pub struct EarlyAllocator<const SIZE: usize> {
    start: usize,
    end: usize,
    b_pos: usize,
    p_pos: usize,
    alloc_count: usize,
}

impl<const SIZE: usize> EarlyAllocator<SIZE> {
    pub const fn new() -> Self {
        Self {
            start: 0,
            end: 0,
            b_pos: 0,
            p_pos: 0,
            alloc_count: 0,
        }
    }
}

impl<const SIZE: usize> BaseAllocator for EarlyAllocator<SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        self.start = start;
        self.end = start + size;
        self.b_pos = start;
        self.p_pos = start + size;
        self.alloc_count = 0;
    }

    fn add_memory(&mut self, start: usize, size: usize) -> allocator::AllocResult {
        if size == 0 {
            return Ok(());
        }
        if self.start == self.end {
            self.init(start, size);
            return Ok(());
        }
        if start + size == self.start && self.b_pos == self.start {
            self.start = start;
            self.b_pos = start;
            return Ok(());
        }
        if start == self.end && self.p_pos == self.end {
            self.end += size;
            self.p_pos += size;
            return Ok(());
        }
        Err(allocator::AllocError::MemoryOverlap)
    }
}

impl<const SIZE: usize> ByteAllocator for EarlyAllocator<SIZE> {
    fn alloc(
        &mut self,
        layout: core::alloc::Layout,
    ) -> allocator::AllocResult<core::ptr::NonNull<u8>> {
        if layout.size() == 0 {
            return Ok(core::ptr::NonNull::dangling());
        }
        let alloc_start = align_up(self.b_pos, layout.align());
        let alloc_end = alloc_start
            .checked_add(layout.size())
            .ok_or(allocator::AllocError::InvalidParam)?;
        if alloc_end > self.p_pos {
            return Err(allocator::AllocError::NoMemory);
        }
        self.b_pos = alloc_end;
        self.alloc_count += 1;
        core::ptr::NonNull::new(alloc_start as *mut u8).ok_or(allocator::AllocError::NoMemory)
    }

    fn dealloc(&mut self, pos: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        let _ = pos;
        if layout.size() == 0 {
            return;
        }
        if self.alloc_count > 0 {
            self.alloc_count -= 1;
            if self.alloc_count == 0 {
                self.b_pos = self.start;
            }
        }
    }

    fn total_bytes(&self) -> usize {
        self.p_pos.saturating_sub(self.start)
    }

    fn used_bytes(&self) -> usize {
        if self.alloc_count == 0 {
            0
        } else {
            self.b_pos.saturating_sub(self.start)
        }
    }

    fn available_bytes(&self) -> usize {
        self.p_pos.saturating_sub(self.b_pos)
    }
}

impl<const SIZE: usize> PageAllocator for EarlyAllocator<SIZE> {
    const PAGE_SIZE: usize = SIZE;

    fn alloc_pages(
        &mut self,
        num_pages: usize,
        align_pow2: usize,
    ) -> allocator::AllocResult<usize> {
        if num_pages == 0 {
            return Err(allocator::AllocError::InvalidParam);
        }
        let align = align_pow2.max(SIZE);
        if !align.is_power_of_two() {
            return Err(allocator::AllocError::InvalidParam);
        }
        let size = num_pages
            .checked_mul(SIZE)
            .ok_or(allocator::AllocError::InvalidParam)?;
        let alloc_start = align_down(
            self.p_pos
                .checked_sub(size)
                .ok_or(allocator::AllocError::NoMemory)?,
            align,
        );
        if alloc_start < self.b_pos {
            return Err(allocator::AllocError::NoMemory);
        }
        self.p_pos = alloc_start;
        Ok(alloc_start)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        let size = num_pages.saturating_mul(SIZE);
        if pos == self.p_pos && pos.saturating_add(size) <= self.end {
            self.p_pos += size;
        }
    }

    fn total_pages(&self) -> usize {
        self.end.saturating_sub(self.start) / SIZE
    }

    fn used_pages(&self) -> usize {
        self.end.saturating_sub(self.p_pos) / SIZE
    }

    fn available_pages(&self) -> usize {
        self.p_pos.saturating_sub(self.b_pos) / SIZE
    }
}
