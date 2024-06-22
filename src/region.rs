use super::constants::REGION_SIZE;
use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;


// we need a new block!
// if we have a free block in the store use that
// else we need to allocate a new region
// this gives us 32 new blocks!
//
// Now we cant dealloc a block directly,
// we must dealloc a whole region
//
// we only dealloc regions after defragmentation

pub struct Region {
    ptr: NonNull<u8>,
    layout: Layout,

}

impl Region {
    pub fn default() -> Result<Region, ()> {
        let layout = Layout::from_size_align(REGION_SIZE, BLOCK_SIZE).unwrap();

        Self::new(layout)
    }

    pub fn new(layout: Layout) -> Result<Region, ()> {
        Ok(Region {
            ptr: Self::alloc(layout)?,
            layout,
        })
    }

    pub fn at_offset(&self, offset: usize) -> *const u8 {
        debug_assert!(offset < REGION_SIZE);

        unsafe { self.ptr.as_ptr().add(offset) }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    pub fn get_size(&self) -> usize {
        self.layout.size()
    }

    fn alloc(layout: Layout) -> Result<NonNull<u8>, ()> {
        unsafe {
            let ptr = alloc(layout);

            if ptr.is_null() {
                Err(())
            } else {
                Ok(NonNull::new_unchecked(ptr))
            }
        }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}
