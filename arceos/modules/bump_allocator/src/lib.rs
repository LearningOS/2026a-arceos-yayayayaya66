#![no_std]

use allocator::{AllocError, AllocResult, BaseAllocator, ByteAllocator, PageAllocator};
use core::{alloc::Layout, ptr::NonNull};

/// Early memory allocator.
/// Use it before formal byte and page allocators can work.
///
/// Bytes grow forward, pages grow backward in a shared memory range:
/// `[start | bytes used | free memory | pages used | end]`.
/// Byte allocations, including their padding, are reclaimed together when
/// the last byte allocation is freed. Page allocations are never reclaimed.
pub struct EarlyAllocator<const SIZE: usize> {
    start: usize,
    end: usize,
    byte_pos: usize,
    page_pos: usize,
    allocations: usize,
}

impl<const SIZE: usize> EarlyAllocator<SIZE> {
    pub const fn new() -> Self {
        assert!(SIZE.is_power_of_two());
        Self {
            start: 0,
            end: 0,
            byte_pos: 0,
            page_pos: 0,
            allocations: 0,
        }
    }

    fn align_up(addr: usize, align: usize) -> Option<usize> {
        addr.checked_add(align - 1).map(|addr| addr & !(align - 1))
    }

    fn pages_in(start: usize, end: usize) -> usize {
        let end = end & !(SIZE - 1);
        Self::align_up(start, SIZE)
            .map(|start| end.saturating_sub(start) / SIZE)
            .unwrap_or(0)
    }
}

impl<const SIZE: usize> BaseAllocator for EarlyAllocator<SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        let end = start.checked_add(size).expect("memory region overflow");
        self.start = start;
        self.end = end;
        self.byte_pos = start;
        self.page_pos = end;
        self.allocations = 0;
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        let end = start.checked_add(size).ok_or(AllocError::InvalidParam)?;
        if size == 0 {
            return Err(AllocError::InvalidParam);
        }
        if self.start == self.end {
            self.init(start, size);
        } else if start < self.end && self.start < end {
            return Err(AllocError::MemoryOverlap);
        } else if start == self.end && self.page_pos == self.end {
            self.end = end;
            self.page_pos = end;
        } else if end == self.start && self.allocations == 0 {
            self.start = start;
            self.byte_pos = start;
        } else {
            // One bump range cannot span holes or move past live allocations.
            return Err(AllocError::NoMemory);
        }
        Ok(())
    }
}

impl<const SIZE: usize> ByteAllocator for EarlyAllocator<SIZE> {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        if layout.size() == 0 {
            // An aligned, non-null dangling pointer needs no backing memory.
            return Ok(NonNull::new(layout.align() as *mut u8).unwrap());
        }
        let start =
            Self::align_up(self.byte_pos.max(1), layout.align()).ok_or(AllocError::NoMemory)?;
        let end = start
            .checked_add(layout.size())
            .ok_or(AllocError::NoMemory)?;
        if end > self.page_pos {
            return Err(AllocError::NoMemory);
        }
        let allocations = self
            .allocations
            .checked_add(1)
            .ok_or(AllocError::NoMemory)?;
        self.byte_pos = end;
        self.allocations = allocations;
        Ok(NonNull::new(start as *mut u8).unwrap())
    }

    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        let start = pos.as_ptr() as usize;
        assert!(self.allocations > 0 && start >= self.start && start < self.byte_pos);
        assert!(layout.size() <= self.byte_pos - start);
        assert!(start & (layout.align() - 1) == 0);
        self.allocations -= 1;
        if self.allocations == 0 {
            self.byte_pos = self.start;
        }
    }

    fn total_bytes(&self) -> usize {
        self.end - self.start
    }

    /// Includes alignment padding retained by the byte allocation front.
    fn used_bytes(&self) -> usize {
        self.byte_pos - self.start
    }

    /// Bytes and pages share this free interval.
    fn available_bytes(&self) -> usize {
        self.page_pos - self.byte_pos
    }
}

impl<const SIZE: usize> PageAllocator for EarlyAllocator<SIZE> {
    const PAGE_SIZE: usize = SIZE;

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        if num_pages == 0 || !align_pow2.is_power_of_two() || align_pow2 < SIZE {
            return Err(AllocError::InvalidParam);
        }
        let size = num_pages.checked_mul(SIZE).ok_or(AllocError::NoMemory)?;
        let start = self
            .page_pos
            .checked_sub(size)
            .ok_or(AllocError::NoMemory)?
            & !(align_pow2 - 1);
        if start < self.byte_pos {
            return Err(AllocError::NoMemory);
        }
        self.page_pos = start;
        Ok(start)
    }

    fn dealloc_pages(&mut self, _pos: usize, _num_pages: usize) {
        // Early page allocations are permanent; only the byte front is recycled.
    }

    fn total_pages(&self) -> usize {
        Self::pages_in(self.start, self.end)
    }

    /// Includes whole pages skipped to satisfy a page allocation's alignment.
    fn used_pages(&self) -> usize {
        Self::pages_in(self.page_pos, self.end)
    }

    fn available_pages(&self) -> usize {
        Self::pages_in(self.byte_pos, self.page_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    #[test]
    fn bytes_and_pages_obey_alignment_without_overlapping() {
        let mut allocator = EarlyAllocator::<4096>::new();
        allocator.init(0x1003, 0x9000);
        let bytes = allocator.alloc(layout(17, 32)).unwrap();
        let pages = allocator.alloc_pages(2, 8192).unwrap();
        assert_eq!(bytes.as_ptr() as usize, 0x1020);
        assert_eq!(pages, 0x8000);
        assert!(bytes.as_ptr() as usize + 17 <= pages);
        assert_eq!(allocator.available_bytes(), 0x8000 - 0x1031);
        assert_eq!(allocator.available_pages(), 6);
        assert_eq!(allocator.used_pages(), 2);
    }

    #[test]
    fn last_byte_free_recycles_padding_but_keeps_pages_reserved() {
        let mut allocator = EarlyAllocator::<4096>::new();
        allocator.init(0x1003, 0x9000);
        let page = allocator.alloc_pages(1, 4096).unwrap();
        let first = allocator.alloc(layout(17, 32)).unwrap();
        let second = allocator.alloc(layout(100, 128)).unwrap();
        let available = allocator.available_bytes();
        allocator.dealloc(first, layout(17, 32));
        assert_eq!(allocator.available_bytes(), available);
        allocator.dealloc(second, layout(100, 128));
        assert_eq!(allocator.used_bytes(), 0);
        assert_eq!(allocator.available_bytes(), page - 0x1003);
        assert_eq!(allocator.alloc(layout(17, 32)).unwrap(), first);
        allocator.dealloc_pages(page, 1);
        assert_eq!(allocator.used_pages(), 1);
        assert_eq!(allocator.alloc_pages(1, 4096).unwrap(), page - 4096);
    }

    #[test]
    fn exhaustion_does_not_consume_memory_or_prevent_reuse() {
        let mut allocator = EarlyAllocator::<4096>::new();
        allocator.init(0x1000, 128);
        let block = allocator.alloc(layout(128, 16)).unwrap();
        assert!(matches!(
            allocator.alloc(layout(1, 1)),
            Err(AllocError::NoMemory)
        ));
        assert!(matches!(
            allocator.alloc_pages(1, 4096),
            Err(AllocError::NoMemory)
        ));
        assert_eq!(allocator.used_bytes(), 128);
        allocator.dealloc(block, layout(128, 16));
        assert_eq!(allocator.alloc(layout(128, 16)).unwrap(), block);
    }

    #[test]
    fn page_alignment_applies_even_to_single_page_allocations() {
        let mut allocator = EarlyAllocator::<4096>::new();
        allocator.init(0x1003, 0xeffe);
        assert_eq!(allocator.alloc_pages(1, 0x8000).unwrap(), 0x8000);
        assert_eq!(allocator.used_pages(), 8);
        assert_eq!(allocator.available_pages(), 6);
        assert!(matches!(
            allocator.alloc_pages(7, 4096),
            Err(AllocError::NoMemory)
        ));
        assert_eq!(allocator.alloc_pages(6, 4096).unwrap(), 0x2000);
    }

    #[test]
    fn invalid_and_overflowing_page_requests_leave_the_range_unchanged() {
        let mut allocator = EarlyAllocator::<4096>::new();
        allocator.init(0x1000, 0x4000);
        for (count, align) in [(0, 4096), (1, 0), (1, 3), (1, 2048), (1, 6144)] {
            assert!(matches!(
                allocator.alloc_pages(count, align),
                Err(AllocError::InvalidParam)
            ));
        }
        assert!(matches!(
            allocator.alloc_pages(usize::MAX, 4096),
            Err(AllocError::NoMemory)
        ));
        assert_eq!(allocator.available_bytes(), 0x4000);
        assert_eq!(allocator.alloc_pages(4, 4096).unwrap(), 0x1000);
        assert_eq!(allocator.available_bytes(), 0);
    }

    #[test]
    fn empty_zero_sized_and_top_of_address_space_allocations() {
        let mut allocator = EarlyAllocator::<4096>::new();
        assert!(matches!(
            allocator.alloc(layout(1, 1)),
            Err(AllocError::NoMemory)
        ));
        assert_eq!(allocator.available_pages(), 0);
        let empty = allocator.alloc(layout(0, 64)).unwrap();
        assert_eq!(empty.as_ptr() as usize % 64, 0);
        allocator.dealloc(empty, layout(0, 64));
        assert_eq!(allocator.used_bytes(), 0);
        allocator.init(usize::MAX - 31, 31);
        assert!(matches!(
            allocator.alloc(layout(1, 64)),
            Err(AllocError::NoMemory)
        ));
        assert_eq!(allocator.available_bytes(), 31);
    }

    #[test]
    fn adding_memory_preserves_live_allocations_and_rejects_holes() {
        let mut allocator = EarlyAllocator::<4096>::new();
        allocator.add_memory(0x2000, 4096).unwrap();
        let block = allocator.alloc(layout(16, 16)).unwrap();
        allocator.add_memory(0x3000, 4096).unwrap();
        assert!(matches!(
            allocator.add_memory(0x1000, 4096),
            Err(AllocError::NoMemory)
        ));
        assert!(matches!(
            allocator.add_memory(0x2000, 4096),
            Err(AllocError::MemoryOverlap)
        ));
        assert!(matches!(
            allocator.add_memory(0x6000, 4096),
            Err(AllocError::NoMemory)
        ));
        assert!(matches!(
            allocator.add_memory(usize::MAX, 1),
            Err(AllocError::InvalidParam)
        ));
        allocator.dealloc(block, layout(16, 16));
        allocator.add_memory(0x1000, 4096).unwrap();
        assert_eq!(allocator.total_bytes(), 0x3000);
        assert_eq!(allocator.alloc_pages(1, 4096).unwrap(), 0x3000);
        assert!(matches!(
            allocator.add_memory(0x4000, 4096),
            Err(AllocError::NoMemory)
        ));
        assert_eq!(allocator.total_bytes(), 0x3000);
    }
}
