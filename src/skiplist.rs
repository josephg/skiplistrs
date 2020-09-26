/// This is an implementation of a general purpose skip list. It was originally
/// ported from a version of skiplists intended for efficient string handling
/// found here - https://github.com/josephg/rustrope

/// This implementation is not optimized for strings (there's some string
/// specific features like unicode handling which have been intentionally
/// removed for simplicity). But it does have another somewhat unusual feature -
/// users can specify their own size function, and lookups, inserts and deletes
/// can use their custom length property to specify offsets.


use std::{mem, ptr};
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::alloc::{alloc, dealloc, Layout};
use std::cmp::min;
use std::marker::PhantomData;
use std::iter;

use std::fmt;

use rand::{RngCore, Rng, SeedableRng};
use rand::rngs::SmallRng;

/// The likelyhood a node will have height (n+1) instead of n
const BIAS: u8 = 100; // out of 256.

/// The number of items in each node. Must fit in a u8 thanks to Node.
#[cfg(debug_assertions)]
const NODE_NUM_ITEMS: usize = 10;

#[cfg(not(debug_assertions))]
const NODE_NUM_ITEMS: usize = 100;

/// List operations will move to linear time after NODE_STR_SIZE * 2 ^
/// MAX_HEIGHT length. (With a smaller constant the higher this is). On the flip
/// side, cursors grow linearly with this number; so smaller is marginally
/// better when the contents are smaller.
#[cfg(debug_assertions)]
const MAX_HEIGHT: usize = 5;

#[cfg(not(debug_assertions))]
const MAX_HEIGHT: usize = 10;


const MAX_HEIGHT_U8: u8 = MAX_HEIGHT as u8; // convenience.

pub struct ItemMarker<Item: ListItem> {
    pub(super) ptr: *mut Node<Item>,
    // _phantom: PhantomData<&'a SkipList<C>>
}

// Derive traits don't work here.
impl<Item: ListItem> Clone for ItemMarker<Item> {
    fn clone(&self) -> Self { *self }
}
impl<Item: ListItem> Copy for ItemMarker<Item> {}
impl<Item: ListItem> PartialEq for ItemMarker<Item> {
    fn eq(&self, other: &Self) -> bool { self.ptr == other.ptr }
}
impl<Item: ListItem> Eq for ItemMarker<Item> {}

impl<Item: ListItem> ItemMarker<Item> {
    pub fn null() -> ItemMarker<Item> {
        ItemMarker { ptr: ptr::null_mut() }
    }

    pub fn is_null(self) -> bool {
        self.ptr.is_null()
    }
}

impl<Item: ListItem> Default for ItemMarker<Item> {
    fn default() -> Self { Self::null() }
}

pub trait ListItem: Sized {
    /// Applications which have custom sizes (or do their own
    /// run-length-encoding) can define their own size function for items. When
    /// items are inserted or replaced, the position is specified using the
    /// custom size defined here.
    fn get_usersize(&self) -> usize { 1 }

    fn userlen_of_slice(items: &[Self]) -> usize {
        items.iter().fold(0, |acc, item| {
            acc + Self::get_usersize(item)
        })
    }

    fn split_item(&self, _at: usize) -> (Self, Self) {
        unimplemented!("Cannot insert in the middle of an item - split_item is not defined in trait");
    }
}

// Blanket implementations for some common builtin types, because its impossible
// to add these later. These make every item have a size of 1.
impl ListItem for () {}
impl<X, Y> ListItem for (X, Y) {}
impl<X, Y, Z> ListItem for (X, Y, Z) {}
impl<V> ListItem for Option<V> {}
impl<T, E> ListItem for Result<T, E> {}

impl<X, Y> ListItem for &(X, Y) {}
impl<X, Y, Z> ListItem for &(X, Y, Z) {}
impl<V> ListItem for &Option<V> {}
impl<T, E> ListItem for &Result<T, E> {}

impl ListItem for u8 {}
impl ListItem for i8 {}
impl ListItem for u16 {}
impl ListItem for i16 {}
impl ListItem for u32 {}
impl ListItem for i32 {}
impl ListItem for f32 {}
impl ListItem for f64 {}

impl ListItem for &u8 {}
impl ListItem for &i8 {}
impl ListItem for &u16 {}
impl ListItem for &i16 {}
impl ListItem for &u32 {}
impl ListItem for &i32 {}
impl ListItem for &f32 {}
impl ListItem for &f64 {}

pub trait NotifyTarget<Item: ListItem> {
    /// To turn off bookkeeping related to ItemMarker query lookups. The
    /// optimizer will inline this
    fn notifications_used() -> bool { true }

    fn notify(&mut self, items: &[Item], at_marker: ItemMarker<Item>);
}

impl<Item: ListItem> NotifyTarget<Item> for () {
    fn notifications_used() -> bool { false }
    fn notify(&mut self, _items: &[Item], _at_marker: ItemMarker<Item>) {}
}

/// This represents a single entry in either the nexts pointers list or in an
/// iterator.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SkipEntry<Item: ListItem> {
    /// The node being pointed to.
    node: *mut Node<Item>,

    /// The number of *items* between the start of the current node and the
    /// start of the next node. That means nexts entry 0 contains the length of
    /// the current node.
    skip_usersize: usize,
}

// We can't use #[derive()] here for Copy and Clone due to a bug in the rust
// compiler: https://github.com/rust-lang/rust/issues/26925
impl<Item: ListItem> Copy for SkipEntry<Item> {}
impl<Item: ListItem> Clone for SkipEntry<Item> {
    fn clone(&self) -> Self { *self }
}

impl<Item: ListItem> SkipEntry<Item> {
    fn new_null() -> Self {
        SkipEntry { node: ptr::null_mut(), skip_usersize: 0 }
    }
}



/// The node structure is designed in a very fancy way which would be more at
/// home in C or something like that. The basic idea is that the node structure
/// is fixed size in memory, but the proportion of that space taken up by
/// characters and by the height differ depending on a node's height. This
/// results in a lot of `unsafe` blocks. I think the tradeoff is worth it but I
/// could be wrong here. You probably wouldn't lose much performance in practice
/// by replacing the inline structure with a smallvec - but that would waste
/// memory in small nodes, and require extra pointer indirection on large nodes.
/// It also wouldn't remove all the unsafe here.
///
/// A different representation (which might be better or worse - I can't tell)
/// would be to have the nodes all be the same size in memory and change the
/// *proportion* of the node's memory that is used by the string field vs the
/// next pointers. That might be lighter weight for the allocator because the
/// struct itself would be a fixed size; but I'm not sure if it would be better.
#[repr(C)] // Prevent parameter reordering.
pub(super) struct Node<Item: ListItem> {
    /// We start with the items themselves. Only the first `num_items` of this
    /// list is in use. The user specified length of the items in the node is
    /// stored in nexts[0].skip_items. This is initialized with
    /// Default::default() for the type, but when MaybeUninit completely lands,
    /// it will be possible to make this a tiny bit faster by leaving the list
    /// initially uninitialized.
    items: [MaybeUninit<Item>; NODE_NUM_ITEMS],

    /// Number of items in `items` in use / filled.
    num_items: u8,

    /// Height of nexts array.
    height: u8,

    /// With the heads array as is, we have no way to go from a marker back to a
    /// cursor (which is required to insert at that location in the list). For
    /// that we need to be able to figure out at each level of the nexts
    /// pointers which object points to us, and the offset from that element to
    /// the current element. Anyway, for markers to work we need this.
    parent: *mut Node<Item>,

    // #[repr(align(std::align_of::<SkipEntry>()))]
    
    /// In reality this array has the size of height, allocated using more or
    /// less direct calls to malloc() at runtime based on the randomly generated
    /// size. The size is always at least 1.
    nexts: [SkipEntry<Item>; 0],
}

// Make sure nexts uses correct alignment. This should be guaranteed by repr(C)
// This test will fail if this ever stops being true.
#[test]
fn test_align() {
    struct Item(u8);
    impl ListItem for Item {}
    #[repr(C)] struct Check([SkipEntry<Item>; 0]);
    assert!(mem::align_of::<Check>() >= mem::align_of::<SkipEntry<Item>>());
    // TODO: It'd be good to also check the alignment of the nexts field in Node.
}

fn random_height<R: RngCore>(rng: &mut R) -> u8 {
    let mut h: u8 = 1;
    // Should I use a csrng here? Does it matter?
    while h < MAX_HEIGHT_U8 && rng.gen::<u8>() < BIAS { h+=1; }
    h
}

#[repr(C)]
pub struct SkipList<Item: ListItem, N: NotifyTarget<Item> = ()> {
    // TODO: Consider putting the head item on the heap. For the use case here
    // its almost certainly fine either way. The code feels a bit cleaner if its
    // on the heap (and then iterators will be able to outlast a move of the
    // skiplist parent). But its also very nice having the code run fast for
    // small lists. Most lists are small, and it makes sense to optimize for
    // that.

    // TODO: For safety, pointers in to this structure should be Pin<> if we
    // ever want to hold on to iterators.

    /// The total number of items in the skip list. This is not used internally -
    /// just here for bookkeeping.
    pub(super) num_items: usize,
    /// Size of the list in user specified units.
    pub(super) num_usercount: usize,

    /// The RNG we use to generate node heights. Specifying it explicitly allows
    /// unit tests and randomizer runs to be predictable, which is very helpful
    /// during debugging. I'm still not sure how the type of this should be
    /// specified. Should it be a generic parameter? Box<dyn *>?
    /// ??
    rng: Option<SmallRng>,

    /// The first node is inline. The height is 1 more than the max height we've
    /// ever used. The highest next entry points to {null, total usersize}.
    head: Node<Item>,

    /// This is so dirty. The first node is embedded in SkipList; but we need to
    /// allocate enough room for height to get arbitrarily large. I could insist
    /// on SkipList always getting allocated on the heap, but for small lists its
    /// much better to be on the stack.
    ///
    /// So this struct is repr(C) and I'm just padding out the struct directly.
    /// All accesses should go through head because otherwise I think we violate
    /// aliasing rules.
    _nexts_padding: [SkipEntry<Item>; MAX_HEIGHT],

    _phantom: PhantomData<N>
}


impl<Item: ListItem> Node<Item> {
    // Do I need to be explicit about the lifetime of the references being tied
    // to the lifetime of the node?
    fn nexts(&self) -> &[SkipEntry<Item>] {
        unsafe {
            std::slice::from_raw_parts(self.nexts.as_ptr(), self.height as usize)
        }
    }

    fn nexts_mut(&mut self) -> &mut [SkipEntry<Item>] {
        unsafe {
            std::slice::from_raw_parts_mut(self.nexts.as_mut_ptr(), self.height as usize)
        }
    }

    fn layout_with_height(height: u8) -> Layout {
        Layout::from_size_align(
            mem::size_of::<Node<Item>>() + mem::size_of::<SkipEntry<Item>>() * (height as usize),
            mem::align_of::<Node<Item>>()).unwrap()
    }

    fn alloc_with_height(height: u8) -> *mut Node<Item> {
        assert!(height >= 1 && height <= MAX_HEIGHT_U8);

        unsafe {
            let node = alloc(Self::layout_with_height(height)) as *mut Node<Item>;
            node.write(Node {
                items: uninit_items_array(),
                num_items: 0,
                height,
                parent: ptr::null_mut(),
                nexts: [],
            });

            for next in (*node).nexts_mut() {
                *next = SkipEntry::new_null();
            }

            node
        }
    }

    fn alloc<R: RngCore>(rng: &mut R) -> *mut Node<Item> {
        Self::alloc_with_height(random_height(rng))
    }

    unsafe fn free(p: *mut Node<Item>) {
        ptr::drop_in_place(p); // We could just implement drop here, but this is cleaner.
        dealloc(p as *mut u8, Self::layout_with_height((*p).height));
    }

    fn content_slice(&self) -> &[Item] {
        let slice = &self.items[..self.num_items as usize];
        unsafe { maybeinit_slice_get_ref(slice) }
    }

    // The height is at least 1, so this is always valid.
    fn first_skip_entry<'a>(&self) -> &'a SkipEntry<Item> {
        unsafe { &*self.nexts.as_ptr() }
    }

    fn first_skip_entry_mut<'a>(&mut self) -> &'a mut SkipEntry<Item> {
        unsafe { &mut *self.nexts.as_mut_ptr() }
    }

    // TODO: Rename to len() ?
    fn get_userlen(&self) -> usize {
        self.first_skip_entry().skip_usersize
    }
    
    fn get_next_ptr(&self) -> *mut Node<Item> {
        self.first_skip_entry().node
    }
}

impl<Item: ListItem> Drop for Node<Item> {
    fn drop(&mut self) {
        for item in &mut self.items[0..self.num_items as usize] {
            // Could instead call assume_init() on each item but this is
            // friendlier to the optimizer.
            unsafe { ptr::drop_in_place(item.as_mut_ptr()); }
        }
    }
}

struct NodeIter<'a, Item: ListItem>(Option<&'a Node<Item>>);
impl<'a, Item: ListItem> Iterator for NodeIter<'a, Item> {
    type Item = &'a Node<Item>;

    fn next(&mut self) -> Option<&'a Node<Item>> {
        let prev = self.0;
        if let Some(n) = self.0 {
            *self = NodeIter(unsafe { n.get_next_ptr().as_ref() });
        }
        prev
    }
}

/// This is a set of pointers with metadata into a location in the list needed
/// to skip ahead, delete and insert in items. A cursor is reasonably heavy
/// weight - we fill in and maintain as many entries as the height of the list
/// dictates.
///
/// This is not needed for simply iterating sequentially through nodes and data.
/// For that look at NodeIter.
///
/// Note most/all methods using cursors are unsafe. This is because cursors use
/// raw mutable pointers into the list, so when used the following rules have to
/// be followed:
///
/// - Whenever a write happens (insert/remove/replace), any cursor not passed to
///   the write function is invalid.
/// - While a cursor is held the SkipList struct should be considered pinned and
///   must not be moved or deleted
#[derive(Copy, Clone)]
pub(crate) struct Cursor<Item: ListItem> {
    /// The global user position of the cursor in the entire list. This is used
    /// for when the max seen height increases, so we can populate previously
    /// unused entries in the cursor and in the head node.
    ///
    /// This field isn't strictly necessary - earlier versions tacked this on to
    /// the last item in entries... I'm still not sure the cleanest way to do
    /// this.
    pub(super) userpos: usize,

    /// When the userpos of an entry is 0 (totally valid and useful), a cursor
    /// becomes ambiguous with regard to where exactly its pointing in the
    /// current entry. This is used to resolve that ambiguity.
    pub(super) local_index: usize,

    pub(super) entries: [SkipEntry<Item>; MAX_HEIGHT],

    // TODO: The cursor can't outlive the skiplist, but doing this makes it
    // tricky to pass cursors around in the Skiplist type. There's probably a
    // way out of this mess, but I'm not good enough at rust to figure it out.
    // _marker: PhantomData<&'a SkipList<C>>,
}

impl<Item: ListItem> Cursor<Item> {
    pub(super) fn update_offsets(&mut self, height: usize, by: isize) {
        for i in 0..height {
            unsafe {
                // This is weird but makes sense when you realise the nexts in
                // the cursor are pointers into the elements that have the
                // actual pointers.
                // Also adding a usize + isize is awful in rust :/
                let skip = &mut (*self.entries[i].node).nexts_mut()[i].skip_usersize;
                *skip = skip.wrapping_add(by as usize);
            }
        }
    }

    /// Move a cursor to the start of the next node. Returns the new node (or a
    /// nullptr if this is the end of the list).
    fn advance_node(&mut self) -> *mut Node<Item> {
        unsafe {
            let SkipEntry { node: e, skip_usersize: offset } = self.entries[0];
            // offset tells us how far into the current element we are (in
            // usersize). We need to increment the offsets by the entry's
            // remaining length to get to the start of the next node.
            let advance_by = (*e).get_userlen() - offset;
            let next = (*e).get_next_ptr();
            let height = (*next).height as usize;

            for i in 0..height {
                self.entries[i] = SkipEntry {
                    node: next,
                    skip_usersize: 0
                };
            }

            for i in height..self.entries.len() {
                self.entries[i].skip_usersize += advance_by;
            }

            self.userpos += advance_by;
            self.local_index = 0;

            next
        }
    }

    pub(super) fn is_at_node_end(&self) -> bool {
        self.local_index == unsafe { (*self.here_ptr()).num_items } as usize
    }

    pub(super) fn advance_item(&mut self, height: usize) {
        if self.is_at_node_end() { self.advance_node(); }
        let usersize = unsafe { self.current_item() }.unwrap().get_usersize();

        for entry in &mut self.entries[0..height] {
            entry.skip_usersize += usersize;
        }
        self.userpos += usersize;
        self.local_index += 1;
    }

    pub(super) fn advance_by_items(&mut self, num: usize, height: usize) {
        for _ in 0..num { self.advance_item(height); }
    }

    pub(super) fn move_to_item_start(&mut self, height: usize, offset: usize) {
        for entry in &mut self.entries[0..height] {
            entry.skip_usersize -= offset;
        }
        self.userpos -= offset;
    }

    pub(super) unsafe fn prev_item<'a>(&self) -> Option<&'a Item> {
        let node = &*self.here_ptr();
        if self.local_index == 0 {
            assert_eq!(self.userpos, 0, "Invalid state: Cursor at start of node");
            None
        } else {
            debug_assert!(self.local_index <= node.num_items as usize);
            Some(&*(node.items[self.local_index - 1].as_ptr()))
        }
    }

    pub(super) unsafe fn prev_item_mut<'a>(&mut self) -> Option<&'a mut Item> {
        let node = &mut *self.here_ptr();
        if self.local_index == 0 {
            assert_eq!(self.userpos, 0);
            None
        } else {
            debug_assert!(self.local_index <= node.num_items as usize);
            Some(&mut *(node.items[self.local_index - 1].as_mut_ptr()))
        }
    }

    // Could be Option<NonNull<_>>...
    pub(super) unsafe fn peek_next_item(&self) -> Option<*mut Item> {
        let next = (*self.here_ptr()).get_next_ptr();
        if next.is_null() { None }
        else {
            debug_assert!((*next).num_items > 0);
            Some((*next).items[0].as_mut_ptr())
        }
    }

    pub(super) unsafe fn current_item<'a>(&self) -> Option<&'a Item> {
        let node = &*self.here_ptr();
        if self.local_index < node.num_items as usize {
            // Ok - just return the current item.
            Some(&*(node.items[self.local_index].as_ptr()))
        } else {
            // Peek the first item in the next node.
            self.peek_next_item().map(|ptr| &*ptr)
        }
    }

    // pub(super) unsafe fn current_item_mut<'a>(&mut self) -> Option<&'a mut C::Item> {
    //     let node = &mut *self.here_ptr();
    //     if self.local_index < node.num_items as usize {
    //         // Ok - just return the current item.
    //         Some(&mut *(node.items[self.local_index].as_mut_ptr()))
    //     } else {
    //         // Peek the first item in the next node.
    //         self.peek_next_item().map(|ptr| &mut *ptr)
    //     }
    // }

    /// Get the pointer to the cursor's current node
    pub(super) fn here_ptr(&self) -> *mut Node<Item> {
        self.entries[0].node
    }
}

impl<Item: ListItem> PartialEq for Cursor<Item> {
    /// Warning: This returns false if one cursor is at the end of a node, and
    /// the other at the start of the next node. Almost all code in this library
    /// leaves cursors at the end of nodes, so this shouldn't matter too much in
    /// practice.
    fn eq(&self, other: &Self) -> bool {
        if self.userpos != other.userpos
            || self.local_index != other.local_index {return false; }

        for i in 0..MAX_HEIGHT {
            let a = &self.entries[i];
            let b = &other.entries[i];
            if a.node != b.node || a.skip_usersize != b.skip_usersize { return false; }
        }
        true
    }
}
impl<Item: ListItem> Eq for Cursor<Item> {}

impl<Item: ListItem> fmt::Debug for Cursor<Item> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cursor")
            .field("userpos", &self.userpos)
            .field("local_index", &self.local_index)
            .finish()
    }
}

// None of the rust builtins give me what I want, which is a copy-free iterator
// to owned items in a MaybeUninit array. Eh; its easy enough to make my own.
struct UninitOwnedIter<'a, Item: ListItem, N: NotifyTarget<Item>> {
    // Based on the core slice IterMut implementation.
    ptr: NonNull<Item>,
    end: *mut Item,
    _marker: PhantomData<&'a SkipList<Item, N>>
}

impl<'a, Item: ListItem, N: NotifyTarget<Item>> UninitOwnedIter<'a, Item, N> {
    /// Make a slice we can iterate from and steal data from without dropping
    /// content. This is unsafe:
    ///
    /// - If the iterator isn't fully drained then remaining items will be
    ///   forgotten (they are not dropped).
    /// - The slice passed in here must be initialized or undefined behaviour
    ///   will hit us.
    ///
    /// After iterating, the contents are uninit memory.
    unsafe fn from_slice(slice: &[MaybeUninit<Item>]) -> Self {
        let ptr = slice.as_ptr() as *mut Item; // Safe.
        let end = ptr.add(slice.len());

        UninitOwnedIter {
            ptr: NonNull::new_unchecked(ptr),
            end,
            _marker: PhantomData
        }
    }
}

impl<'a, Item: ListItem, N: NotifyTarget<Item>> Iterator for UninitOwnedIter<'a, Item, N> {
    type Item = Item;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ptr.as_ptr() == self.end {
            None
        } else {
            let ptr = self.ptr;
            self.ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().offset(1)) };
            Some(unsafe { ptr.as_ptr().read() })
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = (self.end as usize - self.ptr.as_ptr() as usize) / mem::size_of::<Item>();
        (size, Some(size))
    }
}

// TODO: Stolen from MaybeUninit::uninit_array. Replace with the real uninit_array when stable.
#[inline(always)]
fn uninit_items_array<T>() -> [MaybeUninit<T>; NODE_NUM_ITEMS] {
    unsafe { MaybeUninit::<[MaybeUninit<T>; NODE_NUM_ITEMS]>::uninit().assume_init() }
}

// TODO: Stolen from MaybeUninit::slice_get_ref. Replace when available.
#[inline(always)]
unsafe fn maybeinit_slice_get_ref<T>(slice: &[MaybeUninit<T>]) -> &[T] {
    // SAFETY: casting slice to a `*const [T]` is safe since the caller guarantees that
    // `slice` is initialized, and`MaybeUninit` is guaranteed to have the same layout as `T`.
    // The pointer obtained is valid since it refers to memory owned by `slice` which is a
    // reference and thus guaranteed to be valid for reads.
    &*(slice as *const [MaybeUninit<T>] as *const [T])
}


impl<Item: ListItem, N: NotifyTarget<Item>> SkipList<Item, N> {
    pub fn new() -> Self {
        SkipList::<Item, N> {
            num_items: 0,
            num_usercount: 0,
            rng: None,
            head: Node {
                items: uninit_items_array(),
                num_items: 0,
                height: 1, // Stores max height of list nodes
                parent: ptr::null_mut(),
                nexts: [],
            },
            _nexts_padding: [SkipEntry::new_null(); MAX_HEIGHT],
            _phantom: PhantomData
        }
    }

    pub fn init_rng_from_seed(&mut self, seed: u64) {
        self.rng = Some(SmallRng::seed_from_u64(seed));
    }

    fn get_rng(&mut self) -> &mut SmallRng {
        // I'm sure there's a nicer way to implement this.
        if self.rng.is_none() {
            // We'll use a stable RNG in debug mode so the tests are stable.
            if cfg!(debug_assertions) {
                self.init_rng_from_seed(123);
            } else {
                self.rng = Some(SmallRng::from_entropy());
            }
        }
        self.rng.as_mut().unwrap()
    }


    pub fn len_user(&self) -> usize {
        self.num_usercount
    }

    pub fn len_items(&self) -> usize {
        self.num_items as usize
    }

    fn node_iter(&self) -> NodeIter<Item> { NodeIter(Some(&self.head)) }
    
    pub fn iter(&self) -> ListItemIter<Item> {
        ListItemIter {
            node: Some(&self.head),
            index: 0,
            remaining_items: self.len_items()
        }
    }

    #[inline(always)]
    pub(super) fn height(&self) -> usize {
        self.head.height as usize
    }

    fn heads_mut(&mut self) -> &mut [SkipEntry<Item>] {
        unsafe {
            std::slice::from_raw_parts_mut(self.head.nexts.as_mut_ptr(), self._nexts_padding.len())
        }
    }

    fn is_head(&self, node: *const Node<Item>) -> bool {
        node as *const _ == &self.head as *const _
    }

    #[inline(always)]
    fn use_parents() -> bool {
        cfg!(debug_assertions) || N::notifications_used()
    }

    /// Walk the list and validate internal constraints. This is used for
    /// testing the structure itself, and should generally not be called by
    /// users.
    pub fn check(&self) {
        // #[cfg(test)]
        {
            // self.print();
            assert!(self.head.height >= 1);
            assert!(self.head.height <= MAX_HEIGHT_U8);

            let head_ptr = &self.head as *const _ as *mut _;
            // let skip_over = self.get_top_entry();
            // println!("Skip over skip chars {}, num bytes {}", skip_over.skip_items, self.num_bytes);

            let mut prev: [*const Node<Item>; MAX_HEIGHT] = [ptr::null(); MAX_HEIGHT];

            let mut iter = [SkipEntry {
                // Bleh.
                node: head_ptr,
                // The skips will store the total distance travelled since the
                // start of this traversal at each height. All the entries above
                // head.height are ignored though.
                skip_usersize: 0
            }; MAX_HEIGHT];

            let mut num_items = 0;
            let mut num_usercount = 0;

            for (_i, n) in self.node_iter().enumerate() {
                // println!("visiting {:?}", n.as_str());
                if !self.is_head(n) { assert!(n.num_items > 0); }
                assert!(n.height <= MAX_HEIGHT_U8);
                assert!(n.num_items as usize <= NODE_NUM_ITEMS);

                // Make sure the number of items matches the count
                let local_count = Item::userlen_of_slice(n.content_slice());
                assert_eq!(local_count, n.get_userlen());

                if Self::use_parents() {
                    let expect_parent = if self.is_head(n) {
                        ptr::null() // The head's parent is null
                    } else if n.height == self.head.height {
                        &self.head as *const _ // Max height nodes point back to head
                    } else {
                        prev[n.height as usize]
                    };

                    assert_eq!(n.parent as *const _, expect_parent, "invalid parent");
                }
                
                for (i, entry) in iter[0..n.height as usize].iter_mut().enumerate() {
                    assert_eq!(entry.node as *const _, n as *const _);
                    assert_eq!(entry.skip_usersize, num_usercount);

                    // println!("replacing entry {:?} with {:?}", entry, n.nexts()[i].node);
                    prev[i] = n;
                    entry.node = n.nexts()[i].node;
                    entry.skip_usersize += n.nexts()[i].skip_usersize;
                }

                num_items += n.num_items as usize;
                num_usercount += n.get_userlen();

                // Check the value returned by the iterator functions matches.
                let (mut normal_iter, local_offset) = self.cursor_at_userpos(num_usercount);
                assert_eq!(local_offset, 0);
                assert_eq!(normal_iter.userpos, num_usercount);

                // Dirty hack. If n has 0-sized elements at the end, the normal
                // cursor won't be at the end...
                if Self::use_parents() {
                    while normal_iter.here_ptr() != n as *const _ as *mut _ {
                        normal_iter.advance_node();
                    }
                    normal_iter.local_index = n.num_items as usize;
                    let node_iter = unsafe { self.cursor_at_node(n, n.get_userlen(), n.num_items as usize) };
                    assert_eq!(normal_iter, node_iter);
                }
            }

            for entry in iter[0..self.height()].iter() {
                // println!("{:?}", entry);
                assert!(entry.node.is_null());
                assert_eq!(entry.skip_usersize, num_usercount);
            }
            
            // println!("self bytes: {}, count bytes {}", self.num_bytes, num_bytes);
            assert_eq!(self.num_items, num_items);
            assert_eq!(self.len_user(), num_usercount);
        }
    }
    
    
    /// Internal function for creating a cursor at a particular location in the
    /// skiplist. The returned cursor contains list of nodes which point past
    /// the specified position, as well as offsets of how far into their
    /// character lists the specified characters are.
    ///
    /// Sometimes a call to iter_at_userpos is ambiguous:
    ///
    /// - The item can contain items with zero usersize. The cursor could point
    ///   to any of them.
    /// - If the location is at the end of a node, it is equally valid to return
    ///   a position at the start of the next node.
    ///
    /// Because its impossible to move backwards in the list, iter_at_userpos
    /// returns the first admissible location with the specified userpos.
    /// 
    /// Returns (cursor, offset into the specified item).
    ///
    /// TODO: This should be Pin<&self>.
    pub(super) fn cursor_at_userpos(&self, target_userpos: usize) -> (Cursor<Item>, usize) {
        assert!(target_userpos <= self.len_user());

        let mut e: *const Node<Item> = &self.head;
        let mut height = self.height() - 1;
        
        let mut offset = target_userpos; // How many more items to skip

        // We're populating the head node pointer to simplify the case when the
        // iterator grows. We could put offset into the skip_usersize but it
        // would only be *mostly* correct, not always correct. (Since cursor
        // entries above height are not updated by insert.)
        let mut cursor = Cursor {
            entries: [SkipEntry {
                node: &self.head as *const _ as *mut _,
                skip_usersize: usize::MAX
            }; MAX_HEIGHT],
            local_index: 0,
            userpos: target_userpos,
            // _marker: PhantomData,
        };

        loop { // while height >= 0
            let en = unsafe { &*e };
            let next = en.nexts()[height];
            let skip = next.skip_usersize;
            if offset > skip {
                // Go right.
                debug_assert!(e == &self.head || en.num_items > 0);
                offset -= skip;
                e = next.node;
                assert!(!e.is_null(), "Internal constraint violation: Reached end prematurely");
            } else {
                // Record this and go down.
                cursor.entries[height] = SkipEntry {
                    skip_usersize: offset,
                    node: e as *mut Node<Item>, // This is pretty gross
                };

                if height == 0 { break; } else { height -= 1; }
            }
        };

        // We should always land within the node we're pointing to.
        debug_assert!(offset <= unsafe { &*cursor.here_ptr() }.get_userlen());

        // We've found the node. Now look for the index within the node.
        let en = unsafe { &*e };
        let mut index = 0;

        while offset > 0 {
            assert!(index < en.num_items as usize);
            
            let usersize = unsafe { &*en.items[index].as_ptr() }.get_usersize();
            if usersize > offset { break; } // We're in the middle of an item.
            offset -= usersize;
            index += 1;
        }
        cursor.local_index = index;

        (cursor, offset)
    }

    /// Create a cursor at the specified node, using the parents infrastructure
    /// to calculate offsets. The offset and local_index parameters should
    /// specify the offset into the current node. They are accepted as-is.
    /// Offset *must* be at an item boundary
    unsafe fn cursor_at_node(&self, n: *const Node<Item>, mut offset: usize, local_index: usize) -> Cursor<Item> {
        assert!(Self::use_parents(), "cursor_at_node not available if notifications are disabled");

        let mut n = n as *mut Node<Item>; // We don't mutate, but we need a mut ptr.

        let mut cursor = Cursor {
            userpos: 0, // We'll set this later.
            local_index,
            entries: [SkipEntry {
                node: &self.head as *const _ as *mut _,
                skip_usersize: usize::MAX
            }; MAX_HEIGHT],
            // _marker: PhantomData
        };

        let mut h = 0;
        loop {
            while h < (*n).height as usize {
                cursor.entries[h] = SkipEntry {
                    node: n,
                    skip_usersize: offset
                };

                h += 1;
            }

            let parent = (*n).parent;
            // Reached the head.
            if parent.is_null() { break; }

            // If we're the same height as the parent its fine.
            debug_assert!((*parent).height as usize > h
                || (self.is_head(parent) && (*parent).height as usize == h));

            // Walk from parent back to n, figuring out the offset.
            let mut c = parent;
            // let walk_height = (*parent).height as usize - 2;
            let walk_height = (*n).height as usize - 1;
            while c != n {
                let elem = (*c).nexts()[walk_height];
                offset += elem.skip_usersize;
                c = elem.node;
            }

            n = parent;
        }

        cursor.userpos = offset;
        cursor
    }

    /// SAFETY: Self must outlast the marker and not have been moved since the
    /// marker was created. Self should really be Pin<>!
    pub(super) unsafe fn cursor_at_marker<P>(&mut self, marker: ItemMarker<Item>, predicate: P) -> Option<(Cursor<Item>, usize)>
    where P: Fn(&Item) -> Option<usize> {
        // The marker gives us a pointer into a node. Find the item.
        let n = marker.ptr;

        let mut offset: usize = 0;
        let mut local_index = None;
        let mut item_offset = 0;
        for (i, item) in (*n).content_slice().iter().enumerate() {
            if let Some(item_offset_) = predicate(item) {
                // offset += item_offset;
                item_offset = item_offset_;
                local_index = Some(i);
                break;
            } else {
                offset += item.get_usersize();
            }
        }

        local_index.map(|local_index| {
            (self.cursor_at_node(n, offset, local_index), item_offset)
        })
    }

    // Internal fn to create a new node at the specified iterator filled with
    // the specified content. The passed cursor should point at the end of the
    // previous node. It will be updated to point to the end of the newly
    // inserted content.
    // unsafe fn insert_node_at(&mut self, cursor: &mut Cursor<Item>, contents: &[C::Item], new_userlen: usize, move_cursor: bool) {
    unsafe fn insert_node_at<I>(&mut self, cursor: &mut Cursor<Item>, contents: &mut I, num_items: usize, move_cursor: bool, notify: &mut N)
            where I: Iterator<Item=Item> {

        // println!("Insert_node_at {} len {}", contents.len(), self.num_bytes);
        // debug_assert_eq!(new_userlen, C::userlen_of_slice(contents));
        assert!(num_items <= NODE_NUM_ITEMS);
        debug_assert!(contents.size_hint().0 >= num_items);

        let new_node_ptr = Node::alloc(self.get_rng());
        let new_node = &mut *new_node_ptr;
        new_node.num_items = num_items as u8;

        for (slot, item) in new_node.items[..num_items].iter_mut().zip(contents) {
            (slot.as_mut_ptr() as *mut Item).write(item); // Write makes sure we don't drop the old value.
        }

        let new_userlen = Item::userlen_of_slice(new_node.content_slice());

        let new_height = new_node.height;
        let new_height_usize = new_height as usize;

        let mut head_height = self.height();
        while head_height < new_height_usize {
            // This seems weird given we're about to overwrite these values
            // below. What we're doing is retroactively setting up the cursor
            // and head pointers *as if* the height had been this high all
            // along. This way we only have to populate the higher head values
            // lazily.
            let total_userlen = self.num_usercount;
            let nexts = self.heads_mut();
            nexts[head_height].skip_usersize = total_userlen;
            cursor.entries[head_height].skip_usersize = cursor.userpos;

            head_height += 1; // This is ugly.
            self.head.height += 1;
        }

        new_node.parent = if new_height_usize == MAX_HEIGHT {
            &self.head as *const _ as *mut _
        } else { cursor.entries[new_height_usize].node };

        for i in 0..new_height_usize {
            let prev_skip = &mut (*cursor.entries[i].node).nexts_mut()[i];
            let new_nexts = new_node.nexts_mut();

            // The new node points to the successor (or null)
            new_nexts[i] = SkipEntry {
                node: prev_skip.node,
                skip_usersize: new_userlen + prev_skip.skip_usersize - cursor.entries[i].skip_usersize
            };

            // The previous node points to the new node
            *prev_skip = SkipEntry {
                node: new_node_ptr,
                skip_usersize: cursor.entries[i].skip_usersize
            };

            // Move the iterator to the end of the newly inserted node.
            if move_cursor {
                cursor.entries[i] = SkipEntry {
                    node: new_node_ptr,
                    skip_usersize: new_userlen
                };
            }
        }

        for i in new_height_usize..head_height {
            (*cursor.entries[i].node).nexts_mut()[i].skip_usersize += new_userlen;
            if move_cursor {
                cursor.entries[i].skip_usersize += new_userlen;
            }
        }

        // Update parents.
        if Self::use_parents() && new_height_usize > 1 {
            let mut n = new_node_ptr;
            let mut skip_height = 0;

            loop {
                n = (*n).nexts_mut()[skip_height].node;
                if n.is_null() || (*n).height >= new_height { break; }
                
                (*n).parent = new_node_ptr;
                skip_height = usize::max(skip_height, (*n).height as usize - 1);
            }
        }
        
        self.num_items += num_items;
        self.num_usercount += new_userlen;
        if move_cursor {
            cursor.userpos += new_userlen;
            cursor.local_index = num_items;
        }

        notify.notify(new_node.content_slice(), ItemMarker {
            ptr: new_node_ptr,
            // _phantom: PhantomData
        });
    }

    // unsafe fn insert_at_iter(&mut self, cursor: &mut Cursor<C>, contents: &[C::Item]) {
    pub(super) unsafe fn insert_at_iter<I>(&mut self, cursor: &mut Cursor<Item>, contents: &mut I, notify: &mut N)
            where I: ExactSizeIterator<Item=Item> {
        // iter specifies where to insert.

        let mut e = cursor.here_ptr();

        // The insertion offset into the destination node.
        assert!(cursor.userpos <= self.num_usercount);
        assert!(cursor.local_index <= (*e).num_items as usize);

        // We might be able to insert the new data into the current node, depending on
        // how big it is.
        let num_inserted_items = contents.len();

        // Can we insert into the current node?
        let mut insert_here = (*e).num_items as usize + num_inserted_items <= NODE_NUM_ITEMS;

        // Can we insert into the start of the successor node?
        if !insert_here && cursor.local_index == (*e).num_items as usize && num_inserted_items <= NODE_NUM_ITEMS {
            // We can insert into the subsequent node if:
            // - We can't insert into the current node
            // - There _is_ a next node to insert into
            // - The insert would be at the start of the next node
            // - There's room in the next node
            if let Some(next) = (*e).first_skip_entry_mut().node.as_mut() {
                if next.num_items as usize + num_inserted_items <= NODE_NUM_ITEMS {
                    cursor.advance_node();
                    e = next;

                    insert_here = true;
                }
            }
        }

        let item_idx = cursor.local_index;
        let e_num_items = (*e).num_items as usize; // convenience.

        if insert_here {
            // println!("insert_here {}", contents);
            // First push the current items later in the array
            let c = &mut (*e).items;
            if item_idx < e_num_items {
                // Can't use copy_within because Item doesn't necessarily
                // implement Copy. Memmove the existing items.
                ptr::copy(
                    &c[item_idx],
                    &mut c[item_idx + num_inserted_items],
                    (*e).num_items as usize - item_idx);
            }

            // Then copy in the new items. Can't memcpy from an iterator, but
            // the optimizer should make this fast.
            let dest_content_slice = &mut c[item_idx..item_idx + num_inserted_items];
            for (slot, item) in dest_content_slice.iter_mut().zip(contents) {
                // Do not drop the old items - they were only moved.
                slot.as_mut_ptr().write(item);
            }
            let dest_content_slice = maybeinit_slice_get_ref(dest_content_slice);

            (*e).num_items += num_inserted_items as u8;
            self.num_items += num_inserted_items;
            let num_inserted_usercount = Item::userlen_of_slice(dest_content_slice);
            self.num_usercount += num_inserted_usercount;

            // .... aaaand update all the offset amounts.
            cursor.update_offsets(self.height(), num_inserted_usercount as isize);

            // Usually the cursor will be discarded after one change, but for
            // consistency of compound edits we'll update the cursor to point to
            // the end of the new content.
            for entry in cursor.entries[0..self.height()].iter_mut() {
                entry.skip_usersize += num_inserted_usercount;
            }
            cursor.userpos += num_inserted_usercount;
            cursor.local_index += num_inserted_items;

            notify.notify(dest_content_slice, ItemMarker {
                ptr: e,
                // _phantom: PhantomData
            });
        } else {
            // There isn't room. We'll need to add at least one new node to the
            // list. We could be a bit more careful here and copy as much as
            // possible into the current node - that would decrease the number
            // of new nodes in some cases, but I don't think the performance
            // difference will be large enough to justify the complexity.

            // If we're not at the end of the current node, we'll need to remove
            // the end of the current node's data and reinsert it later.
            let num_end_items = e_num_items - item_idx;

            let (end_items, _end_usercount) = if num_end_items > 0 {
                // We'll mark the items as deleted from the node, while leaving
                // the data itself there for now to avoid a copy.

                // Note that if we wanted to, it would also be correct (and
                // slightly more space efficient) to pack some of the new
                // string's characters into this node after trimming it.
                let end_items = &(*e).items[item_idx..e_num_items];
                (*e).num_items = item_idx as u8;
                let end_usercount = (*e).get_userlen() - cursor.entries[0].skip_usersize;

                cursor.update_offsets(self.height(), -(end_usercount as isize));

                // We need to trim the size off because we'll add the characters
                // back with insert_node_at.
                self.num_usercount -= end_usercount;
                self.num_items -= num_end_items;

                (Some(end_items), end_usercount)
            } else {
                (None, 0)
            };

            // Now we insert new nodes containing the new character data. The
            // data is broken into pieces with a maximum size of NODE_NUM_ITEMS.
            // As further optimization, we could try and fit the last piece into
            // the start of the subsequent node.
            let mut items_remaining = num_inserted_items;
            while items_remaining > 0 {
                let insert_here = usize::min(items_remaining, NODE_NUM_ITEMS);
                self.insert_node_at(cursor, contents, insert_here, true, notify);
                items_remaining -= insert_here;
            }

            // TODO: Consider recursively calling insert_at_iter() here instead
            // of making a whole new node for the remaining content.
            if let Some(end_items) = end_items {
                // Passing false to indicate we don't want the cursor updated
                // after this - it should remain at the end of the newly
                // inserted content, which is *before* this end bit.
                self.insert_node_at(cursor, &mut UninitOwnedIter::<Item, N>::from_slice(end_items), end_items.len(), false, notify);
            }
        }
    }

    // unsafe fn insert_at_iter(&mut self, cursor: &mut Cursor<C>, contents: &[C::Item]) {
    //     self.insert_at_iter_and_notify(cursor, contents, Self::no_notify);
    // }

    /// Interestingly unlike the original, here we only care about specifying
    /// the number of removed items by counting them. We do not use usersize in
    /// the deleted item count.
    ///
    /// If the deleted content occurs at the start of a node, the cursor passed
    /// here must point to the end of the previous node, not the start of the
    /// current node.
    pub(super) unsafe fn del_at_iter(&mut self, cursor: &Cursor<Item>, mut num_deleted_items: usize) {
        if num_deleted_items == 0 { return; }

        let mut item_idx = cursor.local_index;
        let mut e = cursor.here_ptr();
        while num_deleted_items > 0 {
            // self.print();
            // if cfg!(debug_assertions) { self.check(); }
            if item_idx == (*e).num_items as usize {
                let entry = (*e).first_skip_entry();
                // End of current node. Skip to the start of the next one. We're
                // intentionally not updating the iterator because if we delete
                // a whole node we need the iterator to point to the previous
                // element. And if we only delete here, the iterator doesn't
                // need to be moved.
                e = entry.node;
                if e.is_null() { panic!("Cannot delete past the end of the list"); }
                item_idx = 0;
            }

            let e_num_items = (*e).num_items as usize;
            let removed_here = min(num_deleted_items, e_num_items - item_idx);
            
            let height = (*e).height as usize;
            let removed_userlen;

            if removed_here < e_num_items || e as *const _ == &self.head as *const _ {
                // Just trim the node down.
                let trailing_items = e_num_items - item_idx - removed_here;
                
                let c = &mut (*e).items;

                if mem::needs_drop::<Item>() {
                    for item in &mut c[item_idx..item_idx + removed_here] {
                        ptr::drop_in_place(item.as_mut_ptr());
                    }
                }

                removed_userlen = Item::userlen_of_slice(maybeinit_slice_get_ref(&c[item_idx..item_idx + removed_here]));
                if trailing_items > 0 {
                    ptr::copy(
                        &c[item_idx + removed_here],
                        &mut c[item_idx],
                        trailing_items);
                }

                (*e).num_items -= removed_here as u8;
                self.num_items -= removed_here;
                self.num_usercount -= removed_userlen;

                for s in (*e).nexts_mut() {
                    s.skip_usersize -= removed_userlen;
                }
            } else {
                // Remove the node from the skip list entirely. e should be the
                // next node after the position of the iterator.
                assert_ne!(cursor.here_ptr(), e);

                removed_userlen = (*e).get_userlen();
                let next = (*e).first_skip_entry().node;

                // println!("removing {:?} contents {:?} height {}", e, (*e).content_slice(), height);

                for i in 0..height {
                    let s = &mut (*cursor.entries[i].node).nexts_mut()[i];
                    s.node = (*e).nexts_mut()[i].node;
                    s.skip_usersize += (*e).nexts()[i].skip_usersize - removed_userlen;
                }

                self.num_items -= (*e).num_items as usize;
                self.num_usercount -= removed_userlen;

                // Update parents.
                if Self::use_parents() && height > 1 {
                    let mut n = e;
                    // let new_parent = cursor.entries[height - 1].node;

                    // If you imagine this node as a big building, we need to
                    // update the parent of all the nodes we cast a shadow over.
                    // So, if our height is 3 and the next nodes have heights 1
                    // and 2, they both need new parents.
                    let mut parent_height = 1;
                    let cursor_node = cursor.here_ptr();
                    let cursor_node_height = (*cursor_node).height as usize;
                    let mut new_parent = if height >= cursor_node_height {
                        cursor.entries[parent_height].node
                    } else {
                        cursor_node
                    };

                    loop {
                        n = (*n).nexts_mut()[parent_height - 1].node;
                        if n.is_null() || (*n).height >= height as u8 { break; }
                        let n_height = (*n).height as usize;
                        
                        assert_eq!((*n).parent, e);
                        assert!(n_height >= parent_height - 1);

                        if n_height > parent_height {
                            parent_height = n_height;
                            if n_height >= cursor_node_height {
                                new_parent = cursor.entries[parent_height].node
                            }
                        }
                        
                        (*n).parent = new_parent;
                    }
                }

                Node::free(e);
                e = next;
            }

            for i in height..self.height() {
                let s = &mut (*cursor.entries[i].node).nexts_mut()[i];
                s.skip_usersize -= removed_userlen;
            }

            num_deleted_items -= removed_here;

            // if cfg!(debug_assertions) { self.check(); }
        }
    }


    pub(super) unsafe fn replace_at_iter<I>(&mut self, cursor: &mut Cursor<Item>, mut removed_items: usize, inserted_content: &mut I, notify: &mut N)
            where I: ExactSizeIterator<Item=Item> {
        if removed_items == 0 && inserted_content.len() == 0 { return; }

        // Replace as many items from removed_items as we can with inserted_content.
        let mut replaced_items = min(removed_items, inserted_content.len());
        removed_items -= replaced_items;

        while replaced_items > 0 {
            debug_assert!(inserted_content.len() >= replaced_items);
            let mut e = cursor.here_ptr();
            if cursor.local_index == (*e).num_items as usize {
                // Move to the next item.
                e = cursor.advance_node();
                if e.is_null() { panic!("Cannot replace past the end of the list"); }
            }

            let index = cursor.local_index;

            let e_num_items = (*e).num_items as usize;
            let replaced_items_here = min(replaced_items, e_num_items - index);

            let dest = &mut (*e).items[index..index + replaced_items_here];
            let old_usersize = Item::userlen_of_slice(maybeinit_slice_get_ref(dest));

            // Replace the items themselves. Everything else is commentary.
            // Would prefer to use zip() but it wants ownership of inserted_content :/
            for slot in dest.iter_mut() {
                *slot.as_mut_ptr() = inserted_content.next().unwrap();
            }

            let dest = maybeinit_slice_get_ref(dest);
            let new_usersize = Item::userlen_of_slice(dest);
            let usersize_delta = new_usersize as isize - old_usersize as isize;

            if usersize_delta != 0 {
                cursor.update_offsets(self.height(), usersize_delta);
                // I hate this.
                self.num_usercount = self.num_usercount.wrapping_add(usersize_delta as usize);
            }

            replaced_items -= replaced_items_here;
            // We'll hop to the next Node at the start of the next loop
            // iteration if needed.
            cursor.local_index += replaced_items_here;

            for i in 0..self.height() {
                cursor.entries[i].skip_usersize += new_usersize;
            }
            cursor.userpos += new_usersize;

            notify.notify(dest, ItemMarker {
                ptr: e,
                // _phantom: PhantomData,
            });
        }

        // Ok now one of two things must be true. Either we've run out of
        // items to remove, or we've run out of items to insert.
        if inserted_content.len() > 0 {
            // Insert!
            debug_assert!(removed_items == 0);
            self.insert_at_iter(cursor, inserted_content, notify);
        } else if removed_items > 0 {
            self.del_at_iter(cursor, removed_items);
        }
    }

    pub(super) unsafe fn replace_item(&mut self, cursor: &mut Cursor<Item>, new_item: Item, notify: &mut N) {
        // This could easily be optimized.
        self.replace_at_iter(cursor, 1, &mut iter::once(new_item), notify);

        // self.modify_at(start_userpos, Self::no_notify, |item, offset| {
        //     assert_eq!(offset, 0, "replace_at must modify the entire item");
        //     *item = 
        // })
    }

    // TODO: This is just for debugging. Do not export this.
    pub fn print(&self) where Item: std::fmt::Debug {
        println!("items: {}\tuserlen: {}, height: {}", self.num_items, self.len_user(), self.head.height);

        print!("HEAD:");
        for s in self.head.nexts() {
            print!(" |{} ", s.skip_usersize);
        }
        println!();

        use std::collections::HashMap;
        let mut ptr_to_id = HashMap::new();
        // ptr_to_id.insert(std::ptr::null(), usize::MAX);
        for (i, node) in self.node_iter().enumerate() {
            print!("{}:", i);
            ptr_to_id.insert(node as *const _, i);
            for s in node.nexts() {
                print!(" |{} ", s.skip_usersize);
            }
            print!("      : {:?}", node.content_slice());
            if let Some(id) = ptr_to_id.get(&(node.parent as *const _)) {
                print!(" (parent: {})", id);
            }
            print!(" (pointer: {:?})", node as *const _);

            println!();
        }
    }
}



impl<Item: ListItem, N: NotifyTarget<Item>> SkipList<Item, N> {
    pub fn eq_list<Rhs>(&self, other: &[Rhs]) -> bool where Item: PartialEq<Rhs> {
        let mut pos = 0;
        let other_len = other.len();

        for node in self.node_iter() {
            let my_data = node.content_slice();
            let my_len = my_data.len();

            if pos + my_len > other_len || my_data != &other[pos..pos + my_data.len()] {
                return false
            }
            pos += my_data.len();
        }

        pos == other_len
    }
}

impl<Item: ListItem, N: NotifyTarget<Item>> Drop for SkipList<Item, N> {
    fn drop(&mut self) {
        let mut node = self.head.first_skip_entry().node;
        unsafe {
            while !node.is_null() {
                let next = (*node).first_skip_entry().node;
                Node::free(node);
                node = next;
            }
        }
    }
}


// Only if there's no notification target.
impl<I, Item: ListItem> From<I> for SkipList<Item> where I: ExactSizeIterator<Item=Item> {
    fn from(iter: I) -> SkipList<Item> {
        SkipList::new_from_iter(iter)
    }
}

impl<Item: ListItem, N: NotifyTarget<Item>> Into<Vec<Item>> for &SkipList<Item, N> where Item: Copy {
    fn into(self) -> Vec<Item> {
        let mut content: Vec<Item> = Vec::with_capacity(self.num_items);

        for node in self.node_iter() {
            content.extend(node.content_slice().iter());
        }

        content
    }
}

impl<Item: ListItem, N: NotifyTarget<Item>> fmt::Debug for SkipList<Item, N> where Item: fmt::Debug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<Item: ListItem, N: NotifyTarget<Item>> Default for SkipList<Item, N> {
    fn default() -> Self {
        SkipList::new()
    }
}


pub struct ListItemIter<'a, Item: ListItem> {
    node: Option<&'a Node<Item>>,
    index: usize,
    remaining_items: usize // For size_hint.
}

impl<'a, Item: ListItem> Iterator for ListItemIter<'a, Item> {
    type Item = &'a Item;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(node) = self.node {
            let current = &node.items[self.index];
            self.index += 1;
            if self.index == node.num_items as usize {
                self.index = 0;
                self.node = unsafe { node.get_next_ptr().as_ref() };
            }

            Some(unsafe { &*current.as_ptr() })
        } else { None }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining_items, Some(self.remaining_items))
    }
}


// impl<T: Default + Copy, F: Fn(&T) -> usize> PartialEq for SkipList<T, F> {
//     // This is quite complicated. It would be cleaner to just write a bytes
//     // iterator, then iterate over the bytes of both strings comparing along the
//     // way.
//     // However, this should be faster because it can memcmp().

//     // Another way to implement this would be to rewrite it as a comparison with
//     // an iterator over &str. Then the list vs list comparison would be trivial,
//     // but also we could add comparison functions with a single &str and stuff
//     // very easily.
//     fn eq(&self, other: &SkipList<T, F>) -> bool {
//         if self.num_items != other.num_items
//                 || self.num_chars() != other.num_chars() {
//             return false
//         }

//         let mut other_iter = other.iter().map(|n| { n.as_str() });

//         let mut os = other_iter.next();
//         let mut opos: usize = 0; // Byte offset in os.
//         for n in self.iter() {
//             let s = n.as_str();
//             let mut pos: usize = 0; // Current byte offset in s
//             debug_assert_eq!(s.len(), n.num_bytes as usize);

//             // Walk s.len() bytes through the other list
//             while pos < n.num_bytes as usize {
//                 if let Some(oss) = os {
//                     let amt = min(s.len() - pos, oss.len() - opos);
//                     // println!("iter slen {} pos {} osslen {} amt {}", s.len(), pos, oss.len(), amt);

//                     if &s[pos..pos+amt] != &oss[opos..opos+amt] {
//                         return false
//                     }

//                     pos += amt;
//                     opos += amt;
//                     debug_assert!(opos <= oss.len());

//                     if opos == oss.len() {
//                         os = other_iter.next();
//                         opos = 0;
//                     }
//                 } else {
//                     panic!("Internal string length does not match");
//                 }
//             }
//         }

//         true
//     }
// }
// impl<T: Default + Copy, F: Fn(&T) -> usize> Eq for SkipList<T, F> {}

// impl<T: Default + Copy, F> Clone for SkipList<T, F> where F: Fn(&T) -> usize {
//     fn clone(&self) -> Self {
//         let mut r = SkipList::new(self.get_usersize);
//         r.num_items = self.num_items;
//         let head_str = self.head.as_str();
//         r.head.items[..head_str.len()].copy_from_slice(head_str.as_bytes());
//         r.head.num_bytes = self.head.num_bytes;
//         r.head.height = self.head.height;
        
//         {
//             // I could just edit the overflow memory directly, but this is safer
//             // because of aliasing rules.
//             let head_nexts = r.head.nexts_mut();
//             for i in 0..self.height() {
//                 head_nexts[i].skip_items = self.nexts[i].skip_items;
//             }
//         }

//         let mut nodes = [&mut r.head as *mut Node; MAX_HEIGHT];

//         // The first node the iterator will return is the head. Ignore it.
//         let mut iter = self.iter();
//         iter.next();
//         for other in iter {
//             // This also sets height.
//             let height = other.height;
//             let node = Node::alloc_with_height(height);
//             unsafe {
//                 (*node).num_bytes = other.num_bytes;
//                 let len = other.num_bytes as usize;
//                 (*node).items[..len].copy_from_slice(&other.items[..len]);

//                 let other_nexts = other.nexts();
//                 let nexts = (*node).nexts_mut();
//                 for i in 0..height as usize {
//                     nexts[i].skip_items = other_nexts[i].skip_items;
//                     (*nodes[i]).nexts_mut()[i].node = node;
//                     nodes[i] = node;
//                 }
//             }
//         }

//         r
//     }
// }
