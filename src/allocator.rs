//! Image-sized buffers bypass macOS malloc's large-block cache.
//!
//! libmalloc keeps recently freed large blocks resident for reuse, and on
//! Macs with plenty of RAM that cache is deep. Preparing one photo frees
//! several decode, resize and encode buffers of a few to a few dozen MiB, so
//! a browsing session filled the cache with them and peak memory roughly
//! doubled. Mapping those buffers directly hands them back to the kernel as
//! soon as they are dropped. Smaller allocations still use the system
//! allocator unchanged.
use std::alloc::{GlobalAlloc, Layout, System};

/// Smallest allocation that is mapped directly. Below this, mapping costs
/// more in system calls and page faults than the cache would ever retain.
const MAPPED: usize = 4 * 1024 * 1024;

pub struct Allocator;

fn mapped(layout: Layout) -> bool {
    // Fresh mappings are page-aligned; stricter alignments stay with malloc.
    layout.size() >= MAPPED && layout.align() <= 4096
}

unsafe fn map(size: usize) -> *mut u8 {
    // SAFETY: an anonymous private mapping has no preconditions.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        std::ptr::null_mut()
    } else {
        ptr.cast()
    }
}

// SAFETY: every block is routed by its layout, which callers must pass back
// unchanged, so a block is always released by the allocator that made it.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if mapped(layout) {
            unsafe { map(layout.size()) }
        } else {
            unsafe { System.alloc(layout) }
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // Anonymous mappings are already zero-filled.
        if mapped(layout) {
            unsafe { map(layout.size()) }
        } else {
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if mapped(layout) {
            unsafe { libc::munmap(ptr.cast(), layout.size()) };
        } else {
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller guarantees new_size is a valid size for this alignment.
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        if !mapped(layout) && !mapped(new_layout) {
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        let new = unsafe { self.alloc(new_layout) };
        if !new.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(ptr: *mut u8, len: usize) {
        for i in 0..len {
            unsafe { ptr.add(i).write(i as u8) };
        }
    }
    fn filled(ptr: *mut u8, len: usize) -> bool {
        (0..len).all(|i| unsafe { ptr.add(i).read() } == i as u8)
    }

    #[test]
    fn routes_by_size_and_alignment() {
        assert!(!mapped(Layout::from_size_align(MAPPED - 1, 8).unwrap()));
        assert!(mapped(Layout::from_size_align(MAPPED, 8).unwrap()));
        // Over-aligned blocks need malloc's alignment support.
        assert!(!mapped(Layout::from_size_align(MAPPED, 8192).unwrap()));
    }

    #[test]
    fn mapped_blocks_are_zeroed_aligned_and_writable() {
        let layout = Layout::from_size_align(MAPPED + 3, 4096).unwrap();
        let ptr = unsafe { Allocator.alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 4096, 0);
        assert!((0..layout.size()).all(|i| unsafe { ptr.add(i).read() } == 0));
        fill(ptr, layout.size());
        assert!(filled(ptr, layout.size()));
        unsafe { Allocator.dealloc(ptr, layout) };
    }

    #[test]
    fn realloc_keeps_contents_across_the_threshold() {
        let small = 1024;
        let large = MAPPED * 2;
        let layout = Layout::from_size_align(small, 8).unwrap();
        let ptr = unsafe { Allocator.alloc(layout) };
        fill(ptr, small);
        // Small to mapped, mapped to larger mapped, then back below the threshold.
        let ptr = unsafe { Allocator.realloc(ptr, layout, large) };
        assert!(filled(ptr, small));
        fill(ptr, large);
        let layout = Layout::from_size_align(large, 8).unwrap();
        let ptr = unsafe { Allocator.realloc(ptr, layout, large + MAPPED) };
        assert!(filled(ptr, large));
        let layout = Layout::from_size_align(large + MAPPED, 8).unwrap();
        let ptr = unsafe { Allocator.realloc(ptr, layout, small) };
        assert!(filled(ptr, small));
        unsafe { Allocator.dealloc(ptr, Layout::from_size_align(small, 8).unwrap()) };
    }
}
