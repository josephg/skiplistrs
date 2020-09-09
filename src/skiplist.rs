/// This is an implementation of a general purpose skip list. It was originally
/// ported from a version of skiplists intended for efficient string handling
/// found here - https://github.com/josephg/rustrope

/// This implementation is not optimized for strings (there's some string
/// specific features like unicode handling which have been intentionally
/// removed for simplicity). But it does have another somewhat unusual feature -
/// users can specify their own size function, and lookups, inserts and deletes
/// can use their custom length property to specify offsets.

/// Unlike other rust rope implementations, this implementation should be very
/// fast; but it manages that through heavy use of unsafe pointers and C-style
/// dynamic arrays.

use std::{mem, ptr};
use std::alloc::{alloc, dealloc, Layout};
use std::cmp::min;

use rand::{RngCore, Rng, SeedableRng};
use rand::rngs::SmallRng;

/// The likelyhood a node will have height (n+1) instead of n
// const BIAS: f64 = 0.4; // this is way more accuracy than we need.
const BIAS: u8 = 100; // likelihood out of 256.

/// The number of items in each node. Must fit in a u8 thanks to Node.
// const NODE_NUM_ITEMS: usize = 100;
const NODE_NUM_ITEMS: usize = 100;

/// Rope operations will move to linear time after NODE_STR_SIZE * 2 ^
/// MAX_HEIGHT length. (With a smaller constant the higher this is). On the flip
/// side, cursors grow linearly with this number; so smaller is marginally
/// better when the contents are smaller.
const MAX_HEIGHT: usize = 20;
// const MAX_HEIGHT: usize = env!("MAX_HEIGHT").parse::<usize>().unwrap();

const MAX_HEIGHT_U8: u8 = MAX_HEIGHT as u8; // convenience.

/// The whole list is configured through a single generic trait parameter
pub trait ListConfig {
    type Item: Default + Copy;

    /// Applications which have custom sizes (or do their own
    /// run-length-encoding) can define their own size function for items. When
    /// items are inserted or replaced, the position is specified using the
    /// custom size defined here.
    fn get_usersize(_item: &Self::Item) -> usize { 1 }

    fn userlen_of_slice(items: &[Self::Item]) -> usize {
        items.iter().fold(0, |acc, item| {
            acc + Self::get_usersize(item)
        })
    }

    // type RngType: rand::RngCore = rand::rngs::SmallRng;
    // fn get_rng() -> Self::RngType {
    //     use rand::SeedableRng;
    //     // rand::rngs::SmallRng::from_seed(SeedableRng::seed_from_u64(10))
    //     rand::rngs::SmallRng::from_entropy()
    // }
}

/// This represents a single entry in either the nexts pointers list or in an
/// iterator.
#[derive(Debug)]
struct SkipEntry<C: ListConfig> {
    /// The node being pointed to.
    node: *mut Node<C>,
    /// The number of *items* between the start of the current node and the
    /// start of the next node. That means nexts entry 0 contains the length of
    /// the current node.
    skip_usersize: usize,
}

// Needed due to rust bug: https://github.com/rust-lang/rust/issues/26925
impl<C: ListConfig> Copy for SkipEntry<C> {}
impl<C: ListConfig> Clone for SkipEntry<C> {
    fn clone(&self) -> Self { *self }
}

// The node structure is designed in a very fancy way which would be more at
// home in C or something like that. The basic idea is that the node structure
// is fixed size in memory, but the proportion of that space taken up by
// characters and by the height differ depending on a node's height. This
// results in a lot of `unsafe` blocks. I think the tradeoff is worth it but I
// could be wrong here. You probably wouldn't lose much performance in practice
// by replacing the inline structure with a smallvec - but that would waste
// memory in small nodes, and require extra pointer indirection on large nodes.
// It also wouldn't remove all the unsafe here.

// A different representation (which might be better or worse - I can't tell)
// would be to have the nodes all be the same size in memory and change the
// *proportion* of the node's memory that is used by the string field vs the
// next pointers. That might be lighter weight for the allocator because the
// struct itself would be a fixed size; but I'm not sure if it would be better.

#[repr(C)] // Prevent parameter reordering.
struct Node<C: ListConfig> {
    /// We start with the items themselves. The number of items in use is in
    /// nexts[0].skip_items. This is initialized with Default::default() for the
    /// type. When MaybeUninit completely lands, it will be possible to make
    /// this a tiny bit faster using that instead; and just leave junk in the
    /// array to start.
    items: [C::Item; NODE_NUM_ITEMS],

    // Number of items in `items` in use / filled.
    num_items: u8,

    // Height of nexts array.
    height: u8,

    // #[repr(align(std::align_of::<SkipEntry>()))]
    
    // This array actually has the size of height, allocated using more or less
    // direct calls to malloc(). It would be cleaner to declare it as
    // [SkipEntry; 0], but I haven't done that because we always have at least a
    // height of 1 anyway, and this makes it a bit cheaper to look at the first
    // skipentry item.
    nexts: [SkipEntry<C>; 0],
}

// Make sure nexts uses correct alignment. This should be guaranteed by repr(C)
// This test will fail if this ever stops being true.
#[test]
fn test_align() {
    struct TestConfig;
    impl ListConfig for TestConfig {
        type Item = u8;
        fn get_usersize(_item: &Self::Item) -> usize { 1 }
    }
    #[repr(C)] struct Check([SkipEntry<TestConfig>; 0]);
    assert!(mem::align_of::<Check>() >= mem::align_of::<SkipEntry<TestConfig>>());
    // TODO: It'd be good to also check the alignment of the nexts field in Node.
}

fn random_height<R: RngCore>(rng: &mut R) -> u8 {
    let mut h: u8 = 1;
    // TODO: This is using the thread_local rng, which is secure but doesn't
    // need to be. Check this is actually fast. I don't think it'll make much
    // difference in practice but it might. Also moving to a prng might reduce
    // code size; which might matter for some users.
    // while h < MAX_HEIGHT_U8 && rng.gen_bool(BIAS) { h+=1; }
    while h < MAX_HEIGHT_U8 && rng.gen::<u8>() < BIAS { h+=1; }
    // while h < MAX_HEIGHT_U8 && rand::random::<u8>() < BIAS { h+=1; }
    h
}

#[repr(C)]
pub struct SkipList<C: ListConfig> {
    // TODO: Put this on the heap. For the use case here its almost certainly fine.

    // TODO: For safety, pointers in to this structure should be Pin<> if we
    // ever want to hold on to iterators.

    /// The total number of items in the skip list. This is not used internally -
    /// just here for bookkeeping.
    num_items: usize,
    num_usercount: usize,
    rng: SmallRng,

    // get_usersize: GetUserSize,

    /// The first node is inline. The height is 1 more than the max height we've
    /// ever used. The highest next entry points to {null, total usersize}.
    head: Node<C>,

    /// This is so dirty. The first node is embedded in SkipList; but we need to
    /// allocate enough room for height to get arbitrarily large. I could insist
    /// on SkipList always getting allocated on the heap, but for small lists its
    /// much better to be on the stack.
    ///
    /// So this struct is repr(C) and I'm just padding out the struct directly.
    /// All accesses should go through head because otherwise I think we violate
    /// aliasing rules.
    _nexts_padding: [SkipEntry<C>; MAX_HEIGHT],
}


impl<C: ListConfig> SkipEntry<C> {
    fn new_null() -> Self {
        SkipEntry { node: ptr::null_mut(), skip_usersize: 0 }
    }
}

impl<C: ListConfig> Node<C> {
    // Do I need to be explicit about the lifetime of the references being tied
    // to the lifetime of the node?
    fn nexts(&self) -> &[SkipEntry<C>] {
        unsafe {
            std::slice::from_raw_parts(self.nexts.as_ptr(), self.height as usize)
        }
    }

    fn nexts_mut(&mut self) -> &mut [SkipEntry<C>] {
        unsafe {
            std::slice::from_raw_parts_mut(self.nexts.as_mut_ptr(), self.height as usize)
        }
    }

    fn layout_with_height(height: u8) -> Layout {
        Layout::from_size_align(
            mem::size_of::<Node<C>>() + mem::size_of::<SkipEntry<C>>() * (height as usize),
            mem::align_of::<Node<C>>()).unwrap()
    }

    fn alloc_with_height(height: u8) -> *mut Node<C> {
        //println!("height {} {}", height, max_height());
        assert!(height >= 1 && height <= MAX_HEIGHT_U8);

        unsafe {
            let node = alloc(Self::layout_with_height(height)) as *mut Node<C>;
            (*node) = Node {
                items: [C::Item::default(); NODE_NUM_ITEMS],
                num_items: 0,
                height: height,
                nexts: [],
            };

            for next in (*node).nexts_mut() {
                *next = SkipEntry::new_null();
            }

            node
        }
    }

    fn alloc<R: RngCore>(rng: &mut R) -> *mut Node<C> {
        Self::alloc_with_height(random_height(rng))
    }

    unsafe fn free(p: *mut Node<C>) {
        dealloc(p as *mut u8, Self::layout_with_height((*p).height));
    }

    fn content_slice(&self) -> &[C::Item] {
        &self.items[..self.num_items as usize]
    }

    // The height is at least 1, so this is always valid.
    fn first_skip_entry<'a>(&self) -> &'a SkipEntry<C> {
        unsafe { &*self.nexts.as_ptr() }
    }

    fn first_skip_entry_mut<'a>(&mut self) -> &'a mut SkipEntry<C> {
        unsafe { &mut *self.nexts.as_mut_ptr() }
    }

    // TODO: Rename to len() ?
    fn get_userlen(&self) -> usize {
        self.first_skip_entry().skip_usersize
    }
    
    fn get_next_ptr(&self) -> *mut Node<C> {
        self.first_skip_entry().node
    }

    /// I dunno where this logic should live, but we want to get the index of
    /// the item at the specified offset into the node (and the offset into the
    /// item).
    /// 
    /// If the offset lands between items, we could return either the previous or next item.
    /// 
    /// Returns (index, item_offset).
    fn get_iter_idx(&self, mut usersize_offset: usize, stick_end: bool) -> (usize, usize) {
        if usersize_offset == 0 { return (0, 0); }

        for (i, item) in self.content_slice().iter().enumerate() {
            let usersize = C::get_usersize(item);
            if usersize > usersize_offset {
                return (i, usersize_offset);
            } else if usersize == usersize_offset {
                return if stick_end { (i, usersize_offset) } else { (i+1, 0) }
            } else {
                usersize_offset -= usersize;
            }
        }
        panic!("Could not find requested offset within the node");
    }
    

    // fn mut_next<'a>(&mut self, i: usize) -> &'a mut SkipEntry {
    //     assert!(i < self.height);
    //     unsafe { &mut *self.nexts.as_mut_ptr() }
    // }
}

struct NodeIter<'a, C: ListConfig>(Option<&'a Node<C>>);
impl<'a, C: ListConfig> Iterator for NodeIter<'a, C> {
    type Item = &'a Node<C>;

    fn next(&mut self) -> Option<&'a Node<C>> {
        let prev = self.0;
        if let Some(n) = self.0 {
            *self = NodeIter(unsafe { n.first_skip_entry().node.as_ref() });
        }
        prev
    }
}

// TODO: Add a phantom lifetime reference to the skip list root for safety.
#[derive(Copy, Clone, Debug)]
struct Cursor<C: ListConfig> {

    entries: [SkipEntry<C>; MAX_HEIGHT],

    // This isn't strictly necessary. Earlier versions tacked this on to the
    // last item in entries... I'm still not sure the cleanest way to do this.
    userpos: usize,

    // / The offset into the pointed item
    // item_offset: usize,
}

impl<C: ListConfig> Cursor<C> {
    fn update_offsets(&mut self, height: usize, by: isize) {
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
    fn advance_node(&mut self) -> *mut Node<C> {
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

            next
        }
    }

    fn here_ptr(&self) -> *mut Node<C> {
        self.entries[0].node
    }
}

impl<C: ListConfig> SkipList<C> {
    pub fn new() -> Self {
        SkipList::<C> {
            num_items: 0,
            num_usercount: 0,
            rng: SmallRng::seed_from_u64(123),
            // get_usersize: get_usersize,
            head: Node {
                items: [C::Item::default(); NODE_NUM_ITEMS],
                num_items: 0,
                height: 1, // Stores max height of list nodes
                nexts: [],
            },
            _nexts_padding: [SkipEntry::new_null(); MAX_HEIGHT],
        }
    }

    pub fn new_from_slice(s: &[C::Item]) -> Self {
        let mut rope = Self::new();
        rope.insert_at(0, s);
        rope
    }

    // fn head(&self) -> Option<&Node> {
    //     unsafe { self.head.nexts[0].next() }
    // }

    // fn get_top_entry(&self) -> SkipEntry<C> {
    //     self.head.nexts()[self.head.height as usize - 1]
    // }

    pub fn get_userlen(&self) -> usize {
        self.num_usercount
        // self.get_top_entry().skip_usersize
    }

    fn iter(&self) -> NodeIter<C> { NodeIter(Some(&self.head)) }
    
    pub fn len_items(&self) -> usize {
        self.num_items as usize
    }

    fn heads_mut(&mut self) -> &mut [SkipEntry<C>] {
        unsafe {
            std::slice::from_raw_parts_mut(self.head.nexts.as_mut_ptr(), MAX_HEIGHT)
        }
    }

    // fn new() -> Self {
    //     SkipList::new()
    // }


    // fn slice(&self, pos: usize, len: usize) -> Result<String, RopeError> {
    //        unimplemented!();
       // }

    // pub fn to_vec(&self) -> Vec { self.into() }

    pub fn check(&self) {
        // #[cfg(test)]
        {
            assert!(self.head.height >= 1);
            assert!(self.head.height <= MAX_HEIGHT_U8);

            // let skip_over = self.get_top_entry();
            // println!("Skip over skip chars {}, num bytes {}", skip_over.skip_items, self.num_bytes);
            // assert!(skip_over.skip_items <= self.num_items as usize);
            // assert!(skip_over.node.is_null());

            let mut iter = [SkipEntry {
                // Bleh.
                node: &self.head as *const Node<C> as *mut Node<C>,
                // The skips will store the total distance travelled since the
                // start of this traversal at each height. All the entries above
                // head.height are ignored though.
                skip_usersize: 0
            }; MAX_HEIGHT];

            let mut num_items = 0;
            let mut num_usercount = 0;

            for n in self.iter() {
                // println!("visiting {:?}", n.as_str());
                assert!((n as *const Node<C> == &self.head as *const Node<C>) || n.num_items > 0);
                assert!(n.height <= MAX_HEIGHT_U8);
                assert!(n.num_items as usize <= NODE_NUM_ITEMS);

                // Make sure the number of items matches the count
                let local_count = C::userlen_of_slice(&n.items[0..n.num_items as usize]);
                assert_eq!(local_count, n.get_userlen());

                // assert_eq!(n.as_str().chars().count(), n.num_chars());
                for (i, entry) in iter[0..n.height as usize].iter_mut().enumerate() {
                    assert_eq!(entry.node as *const Node<C>, n as *const Node<C>);
                    assert_eq!(entry.skip_usersize, num_usercount);

                    // println!("replacing entry {:?} with {:?}", entry, n.nexts()[i].node);
                    entry.node = n.nexts()[i].node;
                    entry.skip_usersize += n.nexts()[i].skip_usersize;
                }

                num_items += n.num_items as usize;
                num_usercount += n.get_userlen();
            }

            for entry in iter[0..self.head.height as usize].iter() {
                // println!("{:?}", entry);
                assert!(entry.node.is_null());
                assert_eq!(entry.skip_usersize, num_usercount);
            }
            
            // println!("self bytes: {}, count bytes {}", self.num_bytes, num_bytes);
            assert_eq!(self.num_items, num_items);
            // assert_eq!(self.num_usercount, num_usercount);
            assert_eq!(self.get_userlen(), num_usercount);
        }
    }
    
    
    /// Internal function for creating a cursor at a particular location in the
    /// skiplist. The returned cursor is a list of nodes which point past the
    /// specified position, as well as offsets of how far into their character
    /// lists the specified characters are.
    /// 
    /// Note this does not calculate the index and offset in the current node.
    ///
    /// TODO: This should be Pin<&self>.
    fn iter_at_userpos(&self, target_userpos: usize) -> Cursor<C> {
        assert!(target_userpos <= self.get_userlen());

        let mut e: *const Node<C> = &self.head;
        let mut height = self.head.height as usize - 1;
        
        let mut offset = target_userpos; // How many more items to skip

        // We're populating it like this so the cursor will remain valid even if
        // new items (with a larger max height) are inserted.
        let mut iter = Cursor {
            entries: [SkipEntry {
                node: &self.head as *const _ as *mut _,
                skip_usersize: offset
            }; MAX_HEIGHT],
            userpos: target_userpos,
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
                assert!(!e.is_null(), "Internal constraint violation: Reached rope end prematurely");
            } else {
                // Record this and go down.
                iter.entries[height] = SkipEntry {
                    skip_usersize: offset,
                    node: e as *mut Node<C>, // This is pretty gross
                };

                if height == 0 { break; } else { height -= 1; }
            }
        };

        assert!(offset <= NODE_NUM_ITEMS);
        iter
    }

    // Internal fn to create a new node at the specified iterator filled with
    // the specified content. The passed cursor should point at the end of the
    // previous node. It will be updated to point to the end of the newly
    // inserted content.
    unsafe fn insert_node_at(&mut self, cursor: &mut Cursor<C>, contents: &[C::Item], new_userlen: usize) {
        // println!("Insert_node_at {} len {}", contents.len(), self.num_bytes);
        debug_assert_eq!(new_userlen, C::userlen_of_slice(contents));
        assert!(contents.len() <= NODE_NUM_ITEMS);
        // assert_eq!(iter.entries[0].skip_usersize, 0);

        let new_node = Node::alloc(&mut self.rng);
        (*new_node).num_items = contents.len() as u8;
        (*new_node).items[..contents.len()].copy_from_slice(contents);
        let new_height = (*new_node).height;

        let mut head_height = self.head.height as usize;
        let new_height_usize = new_height as usize;
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

            // nexts[head_height] = nexts[head_height - 1];
            // iter.entries[head_height] = iter.entries[head_height - 1];
            head_height += 1;
            self.head.height += 1;
        }

        // self.head.height = head_height as u8;

        // if head_height <= new_height_usize {
        //     while head_height <= new_height_usize {
        //         // The highest element in the head's nexts is 1 + the height of
        //         // the max node we've ever had. It points past the end of the
        //         // list.
        //         let nexts = self.heads_mut();
        //         nexts[head_height] = nexts[head_height - 1];
        //         iter.entries[head_height] = iter.entries[head_height - 1];
        //         head_height += 1;
        //     }

        //     self.head.height = head_height as u8;
        // }

        for i in 0..new_height_usize {
            let prev_skip = &mut (*cursor.entries[i].node).nexts_mut()[i];
            let new_nexts = (*new_node).nexts_mut();

            new_nexts[i] = SkipEntry {
                node: prev_skip.node,
                skip_usersize: new_userlen + prev_skip.skip_usersize - cursor.entries[i].skip_usersize
            };

            *prev_skip = SkipEntry {
                node: new_node,
                skip_usersize: cursor.entries[i].skip_usersize
            };

            // & move the iterator to the end of the newly inserted node.
            cursor.entries[i] = SkipEntry {
                node: new_node,
                skip_usersize: new_userlen
            };
        }

        for i in new_height_usize..head_height {
            (*cursor.entries[i].node).nexts_mut()[i].skip_usersize += new_userlen;
            cursor.entries[i].skip_usersize += new_userlen;
        }

        // self.nexts[self.head.height as usize - 1].skip_items += new_userlen;
        self.num_items += contents.len();
        self.num_usercount += new_userlen;
        cursor.userpos += new_userlen;
    }

    unsafe fn insert_at_iter(&mut self, cursor: &mut Cursor<C>, mut item_idx: usize, contents: &[C::Item]) {
        // iter specifies where to insert.

        let mut e = cursor.here_ptr();
        // The insertion offset into the destination node.
        // let mut offset: usize = iter.entries[0].skip_usersize;
        // assert!(offset <= (*e).nexts()[0].skip_usersize);
        assert!(item_idx <= (*e).num_items as usize);

        // We might be able to insert the new data into the current node, depending on
        // how big it is.
        let num_inserted_items = contents.len();
        let num_inserted_usercount = C::userlen_of_slice(contents);

        // Can we insert into the current node?
        let mut insert_here = (*e).num_items as usize + num_inserted_items <= NODE_NUM_ITEMS;

        // Can we insert into the start of the successor node?
        if !insert_here && item_idx == (*e).num_items as usize && num_inserted_items <= NODE_NUM_ITEMS {
            // We can insert into the subsequent node if:
            // - We can't insert into the current node
            // - There _is_ a next node to insert into
            // - The insert would be at the start of the next node
            // - There's room in the next node
            if let Some(next) = (*e).first_skip_entry_mut().node.as_mut() {
                if next.num_items as usize + num_inserted_items <= NODE_NUM_ITEMS {
                    // offset = 0; offset_bytes = 0;
                    item_idx = 0;
                    // for i in 0..next.height {
                    //     // tree offset nodes aren't used here; but they might be referred to by the caller.
                    //     iter.entries[i as usize] = SkipEntry {
                    //         node: next,
                    //         skip_usersize: 0
                    //     };
                    // }
                    cursor.advance_node();
                    e = next;

                    insert_here = true;
                }
            }
        }

        let e_num_items = (*e).num_items as usize; // convenience.

        if insert_here {
            // println!("insert_here {}", contents);
            // First push the current items later in the array
            // let c = (*e).content_mut();
            let c = &mut (*e).items;
            if item_idx < e_num_items {
                c[..].copy_within(item_idx..e_num_items,
                    item_idx + num_inserted_items);
                // ptr::copy(
                //     &c[item_idx],
                //     &mut c[item_idx + num_inserted_items],
                //     e_num_items - item_idx);
            }

            // Then copy in the new items
            c[item_idx..item_idx + num_inserted_items].copy_from_slice(contents);
            // ptr::copy_nonoverlapping(
            //     &contents.as_bytes()[0],
            //     &mut c[offset_bytes],
            //     num_inserted_bytes
            // );

            (*e).num_items += num_inserted_items as u8;
            self.num_items += num_inserted_items;
            self.num_usercount += num_inserted_usercount;

            // .... aaaand update all the offset amounts.
            cursor.update_offsets(self.head.height as usize, num_inserted_usercount as isize);

            // TODO: For consistency we might want to think about updating the
            // iterator here.
        } else {
            // There isn't room. We'll need to add at least one new node to the
            // list. We could be a bit more careful here and copy as much as
            // possible into the current node - that would decrease the number
            // of new nodes in some cases, but I don't think the performance
            // difference will be large enough to justify the complexity.

            // If we're not at the end of the current node, we'll need to remove
            // the end of the current node's data and reinsert it later.
            let num_end_items = e_num_items - item_idx;
            // let mut num_end_usercount: usize = 0;
            let (end_items, end_usercount) = if num_end_items > 0 {
                // We'll mark the items as deleted from the node, while leaving
                // the data itself there for now to avoid a copy.

                // Note that if we wanted to, it would also be correct (and
                // slightly more space efficient) to pack some of the new
                // string's characters into this node after trimming it.
                let end_items = &(*e).items[item_idx..e_num_items];
                (*e).num_items = item_idx as u8;
                let end_usercount = (*e).get_userlen() - cursor.entries[0].skip_usersize;

                cursor.update_offsets(self.head.height as usize, -(end_usercount as isize));

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
            // the start of the subsequent node. That optimization hasn't been
            // added.
            
            for chunk in contents.chunks(NODE_NUM_ITEMS) {
                let userlen = C::userlen_of_slice(chunk);
                self.insert_node_at(cursor, chunk, userlen);
            }

            // TODO: Consider recursively calling insert_at_iter() here instead
            // of making a whole new node for the remaining content.
            if let Some(end_items) = end_items {
                self.insert_node_at(cursor, end_items, end_usercount);
            }
        }
    }

    /// Interestingly unlike the original, here we only care about specifying
    /// the number of removed items by counting them. We do not use usersize in
    /// the deleted item count.
    ///
    /// If the deleted content occurs at the start of a node, the cursor passed
    /// here must point to the end of the previous node, not the start of the
    /// current node.
    unsafe fn del_at_iter(&mut self, cursor: &mut Cursor<C>, mut item_idx: usize, mut num_deleted_items: usize) {
        if num_deleted_items == 0 { return; }

        // let mut offset = iter.entries[0].skip_usersize;
        let mut e = cursor.here_ptr();
        while num_deleted_items > 0 {
            // if offset == s.skip_usersize {
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
                // offset = 0;
            }

            // let num_chars = (*e).num_chars();
            // let removed = min(length, num_chars - offset);
            // assert!(removed > 0);
            let e_num_items = (*e).num_items as usize;
            let removed_here = min(num_deleted_items, e_num_items - item_idx);
            // let removed_userlen = self.userlen_of_slice(&(*e).items[item_idx..item_idx + removed_here]);
            
            let height = (*e).height as usize;
            let removed_userlen;

            if removed_here < e_num_items || e as *const _ == &self.head as *const _ {
                // Just trim the node down.
                // let s = (*e).as_str();
                // let leading_bytes = str_get_byte_offset(s, offset);
                // let removed_bytes = str_get_byte_offset(&s[leading_bytes..], removed);
                // let trailing_bytes = (*e).num_bytes as usize - leading_bytes - removed_bytes;
                let trailing_items = e_num_items - item_idx - removed_here;
                
                let c = &mut (*e).items;
                removed_userlen = C::userlen_of_slice(&c[item_idx..item_idx + removed_here]);
                if trailing_items > 0 {
                    c[..].copy_within(item_idx + removed_here..e_num_items, item_idx);
                    // ptr::copy(
                    //     &c[leading_bytes + removed_bytes],
                    //     &mut c[leading_bytes],
                    //     trailing_bytes);
                }

                // (*e).num_bytes -= removed_bytes as u8;
                (*e).num_items -= removed_here as u8;
                self.num_items -= removed_here;
                self.num_usercount -= removed_userlen;

                for s in (*e).nexts_mut() {
                    s.skip_usersize -= removed_userlen;
                }
                // removed_userlen
            } else {
                // Remove the node from the skip list entirely. e should be the
                // next node after the position of the iterator.
                assert_ne!(cursor.here_ptr(), e);

                removed_userlen = (*e).get_userlen();
                let next = (*e).first_skip_entry().node;

                for i in 0..(*e).height as usize {
                    let s = &mut (*cursor.entries[i].node).nexts_mut()[i];
                    s.node = (*e).nexts_mut()[i].node;
                    s.skip_usersize += (*e).nexts()[i].skip_usersize - removed_userlen;
                }

                self.num_items -= (*e).num_items as usize;
                self.num_usercount -= removed_userlen;

                Node::free(e);
                e = next;
            }

            for i in height..self.head.height as usize {
                let s = &mut (*cursor.entries[i].node).nexts_mut()[i];
                s.skip_usersize -= removed_userlen;
            }

            // length -= removed;
            num_deleted_items -= removed_here;
        }
    }

    pub fn replace_at(&mut self, mut start_userpos: usize, mut removed_items: usize, mut inserted_content: &[C::Item]) {
        if removed_items == 0 && inserted_content.len() == 0 { return; }

        start_userpos = min(start_userpos, self.get_userlen());

        let mut cursor = self.iter_at_userpos(start_userpos);
        let (mut index, offset) = unsafe { &*cursor.here_ptr() }.get_iter_idx(cursor.entries[0].skip_usersize, false);
        assert_eq!(offset, 0, "Splitting nodes not yet supported");

        // Replace as many items from removed_items as we can with inserted_content.
        unsafe {
            let mut replaced_items = min(removed_items, inserted_content.len());
            removed_items -= replaced_items;

            while replaced_items > 0 {
                let mut e = cursor.here_ptr();
                if index == (*e).num_items as usize {
                    // Move to the next item.
                    e = cursor.advance_node();
                    if e.is_null() { panic!("Cannot replace past the end of the list"); }
                    index = 0;
                }

                let e_num_items = (*e).num_items as usize;
                let replaced_items_here = min(replaced_items, e_num_items - index);

                let old_items = &mut (*e).items[index..index + replaced_items_here];
                let new_items = &inserted_content[0..replaced_items_here];

                // Replace the items themselves.
                old_items.copy_from_slice(new_items);

                // And bookkeeping. Bookkeeping forever.
                let new_usersize = C::userlen_of_slice(new_items);
                let usersize_delta = new_usersize as isize - C::userlen_of_slice(old_items) as isize;
                if usersize_delta != 0 {
                    cursor.update_offsets(self.head.height as usize, usersize_delta)
                }

                inserted_content = &inserted_content[replaced_items_here..];
                replaced_items -= replaced_items_here;
                // We'll hop to the next Node at the start of the next loop
                // iteration if needed.
                index += replaced_items_here;

                for i in 0..self.head.height as usize {
                    cursor.entries[i].skip_usersize += new_usersize;
                    // dbg!(i);
                    // dbg!(cursor.entries[i].skip_usersize);
                }
                cursor.userpos += new_usersize;
            }

            // Ok now one of two things must be true. Either we've run out of
            // items to remove, or we've run out of items to insert.
            if inserted_content.len() > 0 {
                // Insert!
                debug_assert!(removed_items == 0);
                self.insert_at_iter(&mut cursor, index, inserted_content);
            } else if removed_items > 0 {
                self.del_at_iter(&mut cursor, index, removed_items);
            }
        }

        // unsafe { self.insert_at_iter(&mut cursor, index, contents); }
    }

    pub fn insert_at(&mut self, mut userpos: usize, contents: &[C::Item]) {
        if contents.len() == 0 { return; }
        
        userpos = min(userpos, self.get_userlen());
        let mut cursor = self.iter_at_userpos(userpos);
        let (index, offset) = unsafe { &*cursor.here_ptr() }.get_iter_idx(cursor.entries[0].skip_usersize, false);
        assert_eq!(offset, 0, "Splitting nodes not yet supported");
        unsafe { self.insert_at_iter(&mut cursor, index, contents); }
    }

    pub fn del_at(&mut self, mut userpos: usize, num_items: usize) {
        userpos = min(userpos, self.get_userlen());
        // We can't easily trim num_items.
        // num_items = min(length, self.num_chars() - pos);
        if num_items == 0 { return; }

        let mut cursor = self.iter_at_userpos(userpos);
        let (index, offset) = unsafe { &*cursor.here_ptr() }.get_iter_idx(cursor.entries[0].skip_usersize, false);
        assert_eq!(offset, 0, "Splitting nodes not yet supported");

        unsafe { self.del_at_iter(&mut cursor, index, num_items); }
    }
}


impl<C: ListConfig> SkipList<C> where C::Item: PartialEq {
    pub fn eq_list(&self, other: &[C::Item]) -> bool {
        let mut pos = 0;
        let other_len = other.len();

        for node in self.iter() {
            let my_data = node.content_slice();
            let my_len = my_data.len();

            if pos + my_len > other_len || my_data != &other[pos..pos + my_data.len()] {
                return false
            }
            pos += my_data.len();
        }

        return pos == other_len;
    }
}

impl<C: ListConfig> Drop for SkipList<C> {
// impl<T, F: Fn(&T) -> usize> Drop for SkipList<T, F> {
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

// impl<T: Default + Copy, F: Fn(&T) -> usize> PartialEq for SkipList<T, F> {
//     // This is quite complicated. It would be cleaner to just write a bytes
//     // iterator, then iterate over the bytes of both strings comparing along the
//     // way.
//     // However, this should be faster because it can memcmp().

//     // Another way to implement this would be to rewrite it as a comparison with
//     // an iterator over &str. Then the rope vs rope comparison would be trivial,
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

//             // Walk s.len() bytes through the other rope
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

impl<C: ListConfig> From<&[C::Item]> for SkipList<C> {
    fn from(s: &[C::Item]) -> SkipList<C> {
        SkipList::new_from_slice(s)
    }
}

impl<C: ListConfig> From<Vec<C::Item>> for SkipList<C> {
    fn from(s: Vec<C::Item>) -> SkipList<C> {
        SkipList::new_from_slice(s.as_slice())
    }
}

impl<C: ListConfig> Into<Vec<C::Item>> for &SkipList<C> {
    fn into(self) -> Vec<C::Item> {
        let mut content = Vec::with_capacity(self.num_items);

        for node in self.iter() {
            content.extend(node.content_slice().iter());
        }

        content
    }
}

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
//             for i in 0..self.head.height as usize {
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

impl<C: ListConfig> SkipList<C> where C::Item: std::fmt::Debug {
    // TODO: Don't export this.
    pub fn print(&self) {
        println!("items: {}\tuserlen: {}, height: {}", self.num_items, self.get_userlen(), self.head.height);

        print!("HEAD:");
        for s in self.head.nexts() {
            print!(" |{} ", s.skip_usersize);
        }
        println!("");

        for (i, node) in self.iter().enumerate() {
            print!("{}:", i);
            for s in node.nexts() {
                print!(" |{} ", s.skip_usersize);
            }
            println!("      : {:?}", node.content_slice());
        }
    }
}