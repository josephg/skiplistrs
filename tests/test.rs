// These tests are also adapted from the C code tests here:
// https://github.com/josephg/librope/blob/master/test/tests.c

#[cfg(test)]
mod test {

    extern crate skiplist;
    use self::skiplist::*;

    extern crate rand;
    use self::rand::Rng;

    use std::fmt::Debug;

    extern crate testdrop;
    use self::testdrop::{Item as TDItem, TestDrop};

    use std::iter;

    // fn clone<Item, T>(slice: &[T]) -> Vec<Item> where Item: From<T>, T: Copy {
    //     slice.iter().copied().map(|t| Item::from(t)).collect()
    // }

    fn as_item<'a, Item, T>(slice: &'a [T]) -> impl 'a + ExactSizeIterator<Item=Item> where Item: From<T>, T: Copy {
        slice.iter().map(|t| Item::from(*t))
    }

    // This config makes all items take up the same amount of space.
    #[derive(Debug, PartialEq, Eq, Copy, Clone)]
    struct FlatItem(u8);
    impl ListItem for FlatItem {}

    impl PartialEq<u8> for FlatItem {
        fn eq(&self, other: &u8) -> bool {
            self.0 == *other
        }
    }

    impl From<u8> for FlatItem {
        fn from(x: u8) -> Self { FlatItem(x) }
    }

    // Here each item names how much space it takes up, so we can try complex
    // positioning.
    #[derive(Debug, PartialEq, Eq, Copy, Clone)]
    struct SizedItem(u8);
    impl ListItem for SizedItem {
        fn get_usersize(&self) -> usize {
            self.0 as usize
        }

        fn split_item(&self, at: usize) -> (Self, Self) {
            (SizedItem(at as u8), SizedItem(self.0 - at as u8))
        }
    }

    impl PartialEq<u8> for SizedItem {
        fn eq(&self, other: &u8) -> bool {
            self.0 == *other
        }
    }

    impl From<u8> for SizedItem {
        fn from(x: u8) -> Self { SizedItem(x) }
    }


    fn check<'a, Item: ListItem>(list: &SkipList<Item>, expected: &'a [u8])
    where Item: Debug + Copy + PartialEq + From<u8> {
        check2(list, expected)
    }

    fn check2<'a, Item: ListItem, T>(list: &SkipList<Item>, expected: &'a [T])
        where Item: Debug + Copy + PartialEq + From<T>, T: Copy {
        // This is super gross.
        let copy: Vec<Item> = as_item(expected).collect();
        let expected = copy.as_slice();

        list.print();
        list.check();
        assert!(list.eq_list(expected));
        
        let vec: Vec<Item> = list.into();
        assert_eq!(vec, expected);
        assert_eq!(list.len_items(), expected.len());

        assert_eq!(list.len_user(), Item::userlen_of_slice(expected));

        // assert_eq!(list, SkipList::from(expected));
        // assert!(*r == SkipList::from(expected), "Rope comparison fails");

        // let clone = r.clone();
        // // clone.print();
        // clone.check();
        // assert!(*r == clone, "Rope does not equal its clone");
    }

    #[test]
    fn sanity() {
        // Lets start by making sure the eq_list() method works right.
        let list = SkipList::<FlatItem>::new();
        assert!(list.eq_list::<u8>(&[]));
        assert!(!list.eq_list(&[1]));
        check(&list, &[]);
        
        let list = SkipList::<FlatItem>::new_from_slice(&[1,2,3,4]);
        assert!(list.eq_list(&[1,2,3,4]));
        assert!(!list.eq_list(&[1,2,3,5]));
        assert!(!list.eq_list(&[1,2,3]));
        check(&list, &[1,2,3,4]);
    }


    #[test]
    fn simple_edits() {
        let mut list = SkipList::<FlatItem>::new_from_slice(&[1,2,3,4]);
        check(&list, &[1,2,3,4]);
        
        list.del_at(1, 2);
        check(&list, &[1,4]);
        
        list.replace_at(1, 1, as_item(&[5,6,7]));
        check(&list, &[1,5,6,7]);
    }
    
    #[test]
    fn empty_list_has_no_contents() {
        let mut list = SkipList::<FlatItem>::new();
        check(&list, &[]);

        list.insert_at_slice(0, &[]);
        check(&list, &[]);
    }

    #[test]
    fn insert_at_location() {
        let mut list = SkipList::<FlatItem>::new();

        list.insert_at(0, as_item(&[1,1,1]));
        check(&list, &[1,1,1]);
        
        list.insert_at(0, as_item(&[2,2,2]));
        check(&list, &[2,2,2,1,1,1]);
        
        list.insert_at(6, as_item(&[3,3,3]));
        check(&list, &[2,2,2,1,1,1,3,3,3]);
        
        list.insert_at(5, as_item(&[4,4,4]));
        check(&list, &[2,2,2,1,1,4,4,4,1,3,3,3]);
    }

    #[test]
    fn insert_between() {
        let mut list = SkipList::<SizedItem>::new_from_slice(&[5,2]);
        
        list.insert_at(1, as_item(&[10]));
        check(&list, &[1,10,4,2]);
    }

    #[test]
    fn del_at_location() {
        let mut list = SkipList::<FlatItem>::new_from_slice(&[0,1,2,3,4,5,6,7,8]);

        list.del_at(8, 1);
        check(&list, &[0,1,2,3,4,5,6,7]);
        
        list.del_at(0, 1);
        check(&list, &[1,2,3,4,5,6,7]);
        
        list.del_at(5, 1);
        check(&list, &[1,2,3,4,5,7]);
        
        list.del_at(5, 1);
        check(&list, &[1,2,3,4,5]);
        
        list.del_at(0, 5);
        check(&list, &[]);
    }

    // #[test]
    // fn del_past_end_of_string() {
    //     let mut r = SkipList::new();

    //     r.del_at(0, 100);
    //     check(&r, "");

    //     r.insert_at(0, "hi there");
    //     r.del_at(3, 10);
    //     check(&r, "hi ");
    // }

    #[test]
    fn really_long_list() {
        let len: usize = 2000;
        let mut content = Vec::<u8>::new();
        // let mut rng = rand::thread_rng();
        for i in 0..len {
            content.push((i % 100) as u8);
        }

        // let s = random_ascii_string(len);

        let mut list = SkipList::<FlatItem>::new_from_slice(content.as_slice());
        check(&list, content.as_slice());

        // Delete everything but the first and last characters
        list.del_at(1, len - 2);
        check(&list, &[content[0], content[len-1]]);
    }

    #[test]
    fn nonuniform_edits() {
        let mut list = SkipList::<SizedItem>::new();
        check(&list, &[]);
        
        list.insert_at(0, as_item(&[2,1]));
        check(&list, &[2,1]);

        list.insert_at(2, as_item(&[0,0]));
        check(&list, &[2,0,0,1]);
        
        list.insert_at(3, as_item(&[5]));
        check(&list, &[2,0,0,1,5]);
        
        list.del_at(3, 1);
        check(&list, &[2,0,0,1]);

        list.insert_at(2, as_item(&[5,5])); // Inserted items go as far left as possible.
        check(&list, &[2,5,5,0,0,1]);

        list.del_at(12, 2);
        check(&list, &[2,5,5,1]);
    }

    #[test]
    fn modify_item() {
        let mut list = SkipList::<SizedItem>::new_from_slice(&[5,4,3,2,1]);
        list.modify_item_after(5, |item, offset| {
            assert_eq!(offset, 0);
            item.0 = 10;
        });
        check(&list, &[5,10,3,2,1]);

        list.modify_item_after(17, |item, offset| {
            assert_eq!(offset, 2); // And check a non-zero offset.
            item.0 = 1;
        });
        check(&list, &[5,10,1,2,1]);
    }

    #[test]
    fn notify() {
        #[derive(PartialEq)]
        struct N {
            count: u32,
            last: ItemMarker<FlatItem>
        };
        impl NotifyTarget<FlatItem> for N {
            fn notify(&mut self, items: &[FlatItem], at_marker: ItemMarker<FlatItem>) {
                assert_eq!(items, &[1,2,3]);
                self.count += 1; // Count
                self.last = at_marker;
            }
        }

        let mut notify_target = N { count: 0, last: ItemMarker::null() };

        let mut list = SkipList::<FlatItem, N>::new();
        list.notify(&mut notify_target).insert_at(0, as_item(&[1,2,3]));

        assert_eq!(notify_target.count, 1);
        
        let marker = notify_target.last;
        let edit = list.edit_at_marker_exact(&mut notify_target, marker, |item| *item == 2).unwrap();

        assert_eq!(edit.prev_item(), Some(&FlatItem(1)));
        assert_eq!(edit.current_item(), Some(&FlatItem(2)));

        assert!(list.edit_at_marker(&mut notify_target, marker, |_item| None).is_none());
    }



    // Trashy non-performant implementation of the API for randomized testing.
    fn vec_find_userpos<Item: ListItem>(list: &Vec<Item>, target_userpos: usize) -> usize {
        let mut item_pos = 0;
        let mut userpos = 0;
        while userpos != target_userpos {
            assert!(item_pos < list.len(), "Trying to insert past the end");
            let usersize = list[item_pos].get_usersize();
            userpos += usersize;
            assert!(userpos <= target_userpos, "Cannot split items");
            item_pos += 1;
        }
        item_pos
    }

    fn vec_insert_at<Item: ListItem>(list: &mut Vec<Item>, target_userpos: usize, content: &[Item]) where Item: Copy {
        let mut item_pos = vec_find_userpos::<Item>(list, target_userpos);
        
        for item in content {
            // This is O(n^2) because of the moves, but this is testing code and
            // its fine. The old code was more complex to make this fast, but I
            // thats probably overkill here.
            list.insert(item_pos, *item);
            item_pos += 1;
        }
    }

    fn vec_delete_at<Item: ListItem>(list: &mut Vec<Item>, target_userpos: usize, num_items: usize) {
        let item_pos = vec_find_userpos::<Item>(list, target_userpos);

        list.drain(item_pos .. item_pos+num_items);
    }

    fn vec_replace<Item: ListItem>(list: &mut Vec<Item>, target_userpos: usize, removed_items: usize, inserted_content: &[Item]) where Item: Copy {
        vec_delete_at::<Item>(list, target_userpos, removed_items);
        vec_insert_at::<Item>(list, target_userpos, inserted_content);
    }


    use self::rand::{SeedableRng, rngs::SmallRng};

    fn gen_random_data<Item: ListItem>(max_len: usize, rng: &mut SmallRng, gen_item: fn(r: &mut SmallRng) -> Item) -> Vec::<Item> {
        let mut content = Vec::<Item>::new();
        // Sometimes generate strings longer than a single node to stress everything.
        for _ in 0..rng.gen_range(0, max_len) { // This should bias toward smaller inserts.
            content.push(gen_item(rng));
        }

        content
    }

    fn random_edits<Item: ListItem>(gen_item: fn(r: &mut SmallRng) -> Item) where Item: PartialEq + Debug + Copy {
        let mut list = SkipList::<Item>::new();
        let mut vec = Vec::<Item>::new();

        let mut rng = SmallRng::seed_from_u64(321);

        let target_min = 800;
        let target_max = 1200;
        let max_chunk_size = 50;

        for i in 0..1000 {
            let itemlen = vec.len();
            let userlen = Item::userlen_of_slice(vec.as_slice());
            // let len = vec.chars().count();

            println!("i {}: items: {} / user: {}", i, itemlen, userlen);
            
            if itemlen == 0 || (itemlen < target_min && rng.gen::<f32>() < 0.35) {
                // Insert.
                let itempos = rng.gen_range(0, itemlen+1);
                let userpos = Item::userlen_of_slice(&vec[0..itempos]);
                if itemlen > 0 { assert!(userlen > 0); }
                
                let content = gen_random_data::<Item>(max_chunk_size, &mut rng, gen_item);

                println!("insert {} content", content.len());
                list.insert_at_slice(userpos, content.as_slice());
                vec_insert_at::<Item>(&mut vec, userpos, content.as_slice());

                check2(&list, vec.as_slice());
            } else if itemlen > target_max || rng.gen::<f32>() < 0.5 {
                // Delete
                let itempos = rng.gen_range(0, itemlen+1); // Sometimes delete nothing at the end.
                let userpos = Item::userlen_of_slice(&vec[0..itempos]);

                // Again some biasing here would be good.
                let num_deleted_items = std::cmp::min(rng.gen_range(0, max_chunk_size), vec.len() - itempos);

                println!("delete {} items", num_deleted_items);
                list.del_at(userpos, num_deleted_items);
                vec_delete_at::<Item>(&mut vec, userpos, num_deleted_items);

                check2(&list, vec.as_slice());
            } else {
                // Replace something!
                let itempos = rng.gen_range(0, itemlen+1);
                let userpos = Item::userlen_of_slice(&vec[0..itempos]);

                let num_deleted_items = std::cmp::min(rng.gen_range(0, max_chunk_size), vec.len() - itempos);
                let ins_content = gen_random_data::<Item>(max_chunk_size, &mut rng, gen_item);

                println!("replace {} with {} items", num_deleted_items, ins_content.len());
                list.replace_at_slice(userpos, num_deleted_items, ins_content.as_slice());
                vec_replace::<Item>(&mut vec, userpos, num_deleted_items, ins_content.as_slice());

                check2(&list, vec.as_slice());
            }
        }
    }

    #[test]
    fn random_edits_flat() {
        random_edits::<FlatItem>(|rng| FlatItem(rng.gen_range(0, 10)));
    }

    #[test]
    fn random_edits_nonuniform() {
        random_edits::<SizedItem>(|rng| SizedItem(rng.gen_range(0, 10)));
    }


    // use std::marker::PhantomData;
    struct DropItem<'a>(TDItem<'a>);
    impl<'a> ListItem for DropItem<'a> {}

    #[test]
    fn inserted_contents_dropped() {
        let td = TestDrop::new();
        let (id, item) = td.new_item();
        let mut list = SkipList::new_from_iter(iter::once(DropItem(item)));
        
        drop(list);
        td.assert_drop(id);
    }

    #[test]
    fn replaced_contents_dropped() {
        let td = TestDrop::new();
        let mut list = SkipList::<DropItem>::new();
        
        let (id1, item1) = td.new_item();
        list.insert_at(0, iter::once(DropItem(item1)));

        let (id2, item2) = td.new_item();
        list.replace_at(0, 1, iter::once(DropItem(item2)));
        td.assert_drop(id1);

        drop(list);
        td.assert_drop(id2);
    }

    #[test]
    fn deleted_contents_dropped() {
        let td = TestDrop::new();
        let (id, item) = td.new_item();
        let mut list = SkipList::new_from_iter(iter::once(DropItem(item)));

        list.del_at(0, 1);
        td.assert_drop(id);
    }
}
