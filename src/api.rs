// This file contains the public facing editing API for skip lists.

use std::iter;
use {ListConfig, NotificationTarget, SkipList, Cursor, ItemMarker};

pub struct Edit<'a, C: ListConfig, N: NotificationTarget<C> = ()> {
    list: &'a mut SkipList<C, N>,
    cursor: Cursor<C>,
    // item_offset: usize, // Offset into the current item.
    notify: &'a mut N,
}

impl<'a, C: ListConfig, N: NotificationTarget<C>> Edit<'a, C, N> {
    fn dbg_check_cursor_at(&self, userpos: usize, plus_items: usize) {
        if cfg!(debug_assertions) {
            let (mut c2, _) = self.list.cursor_at_userpos(userpos);
            c2.advance_by_items(plus_items, self.list.height());
            assert_eq!(&self.cursor, &c2);
        }
    }

    pub fn del(&mut self, num_items: usize) {
        unsafe { self.list.del_at_iter(&self.cursor, num_items); }

        if cfg!(debug_assertions) {
            let (c2, _) = self.list.cursor_at_userpos(self.cursor.userpos);
            if self.cursor != c2 { panic!("Invalid cursor after delete"); }
        }
    }

    pub fn insert_iter<I>(&mut self, mut contents: I) where I: ExactSizeIterator<Item=C::Item> {
        if contents.len() == 0 { return; }
        let num_inserted_items = contents.len();
        let start_userpos = self.cursor.userpos;

        unsafe {
            self.list.insert_at_iter(&mut self.cursor, &mut contents, &mut self.notify);

            self.dbg_check_cursor_at(start_userpos, num_inserted_items);
        }
    }

    pub fn insert_between_iter<I>(&mut self, offset: usize, mut contents: I) where I: ExactSizeIterator<Item=C::Item> {
        if offset == 0 { return self.insert_iter(contents); }

        let num_inserted_items = contents.len();
        let start_userpos = self.cursor.userpos;

        unsafe {
            let current_item = self.cursor.current_item();
            let (start, end) = C::split_item(current_item.unwrap(), offset);
            // Move the cursor back to the start of the item we're
            // splitting.
            self.cursor.move_to_item_start(self.list.height(), offset);
            // This feels pretty inefficient; but its probably fine.
            self.list.replace_item(&mut self.cursor, start, &mut self.notify);

            // TODO: Consider concatenating end into contents then just call
            // insert_at_iter once.
            self.list.insert_at_iter(&mut self.cursor, &mut contents, &mut self.notify);

            self.dbg_check_cursor_at(start_userpos, num_inserted_items);

            self.list.insert_at_iter(&mut self.cursor, &mut iter::once(end), &mut self.notify);
        }
    }

    pub fn insert(&mut self, item: C::Item) {
        self.insert_iter(iter::once(item));
    }

    pub fn insert_between(&mut self, offset: usize, item: C::Item) {
        self.insert_between_iter(offset, iter::once(item));
    }

    pub fn insert_slice(&mut self, items: &[C::Item]) where C::Item: Copy {
        self.insert_iter(items.iter().copied());
    }

    pub fn replace<I>(&mut self, removed_items: usize, mut inserted_content: I)
    where I: ExactSizeIterator<Item=C::Item> {
        let num_inserted_items = inserted_content.len();
        let start_userpos = self.cursor.userpos;
        
        unsafe { self.list.replace_at_iter(&mut self.cursor, removed_items, &mut inserted_content, &mut self.notify); }

        self.dbg_check_cursor_at(start_userpos, num_inserted_items);
    }

    pub fn prev_item(&self) -> Option<&C::Item> {
        unsafe { self.cursor.prev_item() }
    }

    pub fn current_item(&self) -> Option<&C::Item> {
        unsafe { self.cursor.current_item() }
    }

    pub fn advance_item(&mut self) {
        self.cursor.advance_item(self.list.height());
    }

    pub fn modify_prev_item<F>(&mut self, modify_fn: F) where F: FnOnce(&mut C::Item) {
        let item = unsafe { self.cursor.prev_item_mut() }.expect("Cursor at start of document. Cannot modify prev");

        let old_usersize = C::get_usersize(item);
        modify_fn(item);
        let new_usersize = C::get_usersize(item);

        let usersize_delta = new_usersize as isize - old_usersize as isize;

        if usersize_delta != 0 {
            self.cursor.update_offsets(self.list.height(), usersize_delta);
            self.list.num_usercount = self.list.num_usercount.wrapping_add(usersize_delta as usize);
        }

        self.notify.notify(std::slice::from_ref(item), ItemMarker {
            ptr: self.cursor.here_ptr(),
            // _phantom: PhantomData,
        });
    }

    /// Caveat: This moves the cursor to the next item
    // TODO: Not sure if this function is correct. Needs tests!
    pub fn modify_current_item<F>(&mut self, modify_fn: F) where F: FnOnce(&mut C::Item) {
        self.advance_item();
        self.modify_prev_item(modify_fn);
    }

    pub fn replace_prev_item(&mut self, replacement: C::Item) {
        self.modify_prev_item(|old| *old = replacement);
    }
}

pub trait SimpleApi<'a, C: 'a + ListConfig, N: 'a + NotificationTarget<C>> where Self: Sized {
    fn edit(self, userpos: usize) -> (Edit<'a, C, N>, usize);

    fn edit_exact(self, userpos: usize) -> Edit<'a, C, N>;


    fn replace_at<I: ExactSizeIterator<Item=C::Item>>(self, start_userpos: usize, removed_items: usize, inserted_content: I) {
        self.edit_exact(start_userpos).replace(removed_items, inserted_content);
    }

    fn replace_at_slice(self, start_userpos: usize, removed_items: usize, inserted_content: &[C::Item]) where C::Item: Copy {
        self.replace_at(start_userpos, removed_items, inserted_content.iter().copied());
    }

    fn modify_item_after<F: FnOnce(&mut C::Item, usize)>(self, userpos: usize, modify_fn: F) {
        let (mut edit, offset) = self.edit(userpos);
        edit.modify_current_item(|item| modify_fn(item, offset))
    }

    fn insert_at<I: ExactSizeIterator<Item=C::Item>>(self, userpos: usize, contents: I) {
        let (mut edit, offset) = self.edit(userpos);
        edit.insert_between_iter(offset, contents);
    }

    fn insert_at_slice(self, userpos: usize, contents: &[C::Item]) where C::Item: Copy {
        self.insert_at(userpos, contents.iter().copied())
    }

    fn del_at(self, userpos: usize, num_items: usize) {
        self.edit_exact(userpos).del(num_items)
    }
}

static mut NULL_NOTIFY_TARGET: () = ();

impl<'a, C: 'a + ListConfig> SimpleApi<'a, C, ()> for &'a mut SkipList<C> {
    fn edit(self, userpos: usize) -> (Edit<'a, C>, usize) {
        let (cursor, item_offset) = self.cursor_at_userpos(userpos);
        (Edit { list: self, cursor, notify: unsafe { &mut NULL_NOTIFY_TARGET } }, item_offset)
    }

    fn edit_exact(self, userpos: usize) -> Edit<'a, C> {
        let (cursor, item_offset) = self.cursor_at_userpos(userpos);
        assert_eq!(item_offset, 0, "edit_between landed inside an item");
        Edit { list: self, cursor, notify: unsafe { &mut NULL_NOTIFY_TARGET } }
    }
}

impl<'a, C: 'a + ListConfig, N: 'a + NotificationTarget<C>> SimpleApi<'a, C, N> for (&'a mut SkipList<C, N>, &'a mut N) {
    fn edit(self, userpos: usize) -> (Edit<'a, C, N>, usize) {
        let (cursor, item_offset) = self.0.cursor_at_userpos(userpos);
        (Edit { list: self.0, cursor, notify: self.1 }, item_offset)
    }

    fn edit_exact(self, userpos: usize) -> Edit<'a, C, N> {
        let (cursor, item_offset) = self.0.cursor_at_userpos(userpos);
        assert_eq!(item_offset, 0, "edit_between landed inside an item");
        Edit { list: self.0, cursor, notify: self.1 }
    }
}

// These methods are only available if there's no notification target.
impl<C: ListConfig> SkipList<C> {
    pub fn new_from_iter<I: ExactSizeIterator<Item=C::Item>>(iter: I) -> Self {
        let mut list = Self::new();
        list.insert_at(0, iter);
        list
    }

    pub fn new_from_slice(s: &[C::Item]) -> Self where C::Item: Copy {
        Self::new_from_iter(s.iter().copied())
    }
}

impl<C: ListConfig, N: NotificationTarget<C>> SkipList<C, N> {
    pub fn notify<'a>(&'a mut self, notify: &'a mut N) -> (&'a mut Self, &'a mut N) {
        (self, notify)
    }

    pub fn new_from_iter_n<I: ExactSizeIterator<Item=C::Item>>(notify: &mut N, iter: I) -> Self {
        let mut list = Self::new();
        list.notify(notify).insert_at(0, iter);
        list
    }

    pub fn new_from_slice_n(notify: &mut N, s: &[C::Item]) -> Self where C::Item: Copy {
        Self::new_from_iter_n(notify, s.iter().copied())
    }

    pub fn edit_n<'a>(&'a mut self, notify: &'a mut N, userpos: usize) -> (Edit<C, N>, usize) {
        (self, notify).edit(userpos)
    }

    pub fn edit_between_n<'a>(&'a mut self, notify: &'a mut N, userpos: usize) -> Edit<'a, C, N> {
        (self, notify).edit_exact(userpos)
    }

    pub fn edit_at_marker_exact<'a, P>(&'a mut self, notify: &'a mut N, marker: ItemMarker<C>, predicate: P) -> Option<Edit<'a, C, N>>
    where P: Fn(&C::Item) -> bool {
        unsafe {
            self.cursor_at_marker(marker, |item| if predicate(item) { Some(0) } else { None })
        }.map(move |(cursor, item_offset)| {
            debug_assert_eq!(item_offset, 0, "Internal consistency violation");
            Edit { list: self, cursor, notify }
        })
    }

    pub fn edit_at_marker<'a, P>(&'a mut self, notify: &'a mut N, marker: ItemMarker<C>, predicate: P) -> Option<(Edit<'a, C, N>, usize)>
    where P: Fn(&C::Item) -> Option<usize> {
        unsafe {
            self.cursor_at_marker(marker, predicate)
        }.map(move |(cursor, item_offset)| {
            (Edit { list: self, cursor, notify }, item_offset)
        })
    }
}