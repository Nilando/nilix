use super::alloc_head::AllocHead;
use super::allocate::Allocate;
use super::arena::Arena;
use super::header::Header;
use super::header::Mark;
use super::size_class::SizeClass;
use std::alloc::Layout;
use std::mem::{align_of, size_of};
use std::ptr::write;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

pub struct Allocator {
    head: AllocHead,
    current_mark: Arc<AtomicU8>,
}

impl Allocate for Allocator {
    type Arena = Arena;

    fn new(arena: &Self::Arena) -> Self {
        let current_mark = arena.get_current_mark_ref();

        Self {
            head: AllocHead::new(arena.get_block_store()),
            current_mark,
        }
    }

    fn alloc(&self, layout: Layout) -> Result<NonNull<u8>, ()> {
        let align = std::cmp::max(align_of::<Header>(), layout.align());
        let header_size = size_of::<Header>();
        let padding = (align - (header_size % align)) % align;
        let alloc_size = header_size + padding + layout.size();
        let size_class = SizeClass::get_for_size(alloc_size)?;
        // Alloc size could be greater than u16, causing overflow conversion from (as u16).
        // This is okay though, b/c in that case the object will be SizeClass::Large
        // where the header size is unused. Normally the header size is used,
        // for marking block lines, but large objects are stored in bump blocks.
        let header = Header::new(size_class, alloc_size as u16);

        unsafe {
            let alloc_layout = Layout::from_size_align(alloc_size, align).unwrap();
            let space = self.head.alloc(alloc_layout)?;
            let object_space = space.add(header_size + padding);

            write(space as *mut Header, header);
            Ok(NonNull::new(object_space as *mut u8).unwrap())
        }
    }

    fn get_mark<T>(ptr: NonNull<T>) -> Mark {
        let header_ptr = Self::get_header(ptr);

        Header::get_mark(header_ptr)
    }

    fn set_mark<T>(ptr: NonNull<T>, mark: Mark) {
        let header_ptr = Self::get_header(ptr);

        Header::set_mark(header_ptr, mark)
    }

    fn is_old<T>(&self, ptr: NonNull<T>) -> bool {
        Self::get_mark(ptr) == self.get_current_mark()
    }
}

impl Allocator {
    pub fn get_header<T>(object: NonNull<T>) -> *const Header {
        let align = std::cmp::max(align_of::<Header>(), align_of::<T>());
        let header_size = size_of::<Header>();
        let padding = (align - (header_size % align)) % align;
        let ptr: *mut u8 = object.as_ptr().cast::<u8>();

        debug_assert!((ptr as usize % align) == 0);
        debug_assert!((object.as_ptr() as usize % align_of::<T>()) == 0);

        unsafe { ptr.sub(header_size + padding) as *const Header }
    }

    fn get_current_mark(&self) -> Mark {
        Mark::from(self.current_mark.load(Ordering::SeqCst))
    }
}
#[cfg(test)]
mod tests {
    use super::*; 
    use crate::constants::{BLOCK_SIZE, BLOCK_CAPACITY};
    use crate::arena::Arena;
    use crate::allocate::{Allocate, GenerationalArena};
    use crate::header::{Header, Mark};

    #[test]
    fn hello_alloc() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        let name = "Hello Alloc";
        let layout = Layout::for_value(&name);

        assert_eq!(arena.get_size(), 0);

        allocator.alloc(layout).unwrap();

        assert_eq!(arena.get_size(), BLOCK_SIZE);
    }

    #[test]
    fn alloc_large() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        let data: [usize; BLOCK_SIZE] = [0; BLOCK_SIZE];
        let layout = Layout::for_value(&data);

        assert_eq!(arena.get_size(), 0);

        allocator.alloc(layout).unwrap();
    }

    #[test]
    fn alloc_many_single_bytes() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        let layout = Layout::new::<u8>();

        for _ in 0..100_000 {
            allocator.alloc(layout).unwrap();
        }
    }

    #[test]
    fn alloc_too_big() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        let layout = Layout::from_size_align(std::u32::MAX as usize, 8).unwrap();
        let result = allocator.alloc(layout);
        assert!(result.is_err());
    }

    #[test]
    fn alloc_two_large_arrays() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        let layout = Layout::from_size_align(BLOCK_CAPACITY / 2, 8).unwrap();
        allocator.alloc(layout).unwrap();
        assert_eq!(arena.get_size(), BLOCK_SIZE);
        allocator.alloc(layout).unwrap();
        assert_eq!(arena.get_size(), BLOCK_SIZE * 2);
    }

    #[test]
    fn refresh_arena() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        let layout = Layout::from_size_align(BLOCK_CAPACITY / 2, 8).unwrap();
        for _ in 0..20 {
            allocator.alloc(layout).unwrap();
        }
        assert!(arena.get_size() > 10 * BLOCK_SIZE);
        arena.refresh();
        assert_eq!(arena.get_size(), BLOCK_SIZE);
    }

    #[test]
    fn object_align() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        for i in 0..10 {
            let align: usize = 2_usize.pow(i);
            let layout = Layout::from_size_align(32, align).unwrap();
            let ptr = allocator.alloc(layout).unwrap();

            assert!(((ptr.as_ptr() as usize) % align) == 0)
        }
    }

    #[test]
    fn clone_size_class() {
        // this is just for test coverage
        let foo = SizeClass::get_for_size(69);
        let clone = foo;

        assert!(foo == clone);
    }

    #[test]
    fn large_object_align() {
        let arena = Arena::new();
        let allocator = Allocator::new(&arena);
        let layout = Layout::from_size_align(BLOCK_CAPACITY * 2, 128).unwrap();
        let ptr = allocator.alloc(layout).unwrap();

        assert!(((ptr.as_ptr() as usize) % 128) == 0)
    }

    #[test]
    fn arena_get_size() {
        let arena = Arena::new();
        let alloc = Allocator::new(&arena);

        let small = Layout::new::<u8>();
        let medium = Layout::new::<[u8; 512]>();
        let large = Layout::new::<[u8; 80_000]>();

        let p1: NonNull<u8> = alloc.alloc(small).unwrap();
        let p2: NonNull<[u8; 512]> = alloc.alloc(medium).unwrap().cast();
        let p3: NonNull<[u8; 80_000]> = alloc.alloc(large).unwrap().cast();
        Allocator::set_mark(p1, Mark::Red);
        Allocator::set_mark(p2, Mark::Red);
        Allocator::set_mark(p3, Mark::Red);

        let small_header = Allocator::get_header(p1);
        let med_header = Allocator::get_header(p2);
        let large_header = Allocator::get_header(p3);

        unsafe {
            assert_eq!((&*small_header).get_size_class(), SizeClass::Small);
            assert_eq!((&*med_header).get_size_class(), SizeClass::Medium);
            assert_eq!((&*large_header).get_size_class(), SizeClass::Large);
        }

        let align = std::cmp::max(align_of::<Header>(), large.align());
        let header_size = size_of::<Header>();
        let padding = (align - (header_size % align)) % align;
        let large_size = header_size + padding + large.size();

        assert_eq!(arena.get_size(), (BLOCK_SIZE + large_size));
    }
}
