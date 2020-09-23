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

    // This config makes all items take up the same amount of space.
    struct TestConfigFlat;
    impl ListConfig for TestConfigFlat {
        type Item = u8;
    }

    // Here each item names how much space it takes up, so we can try complex
    // positioning.
    struct TestConfigSized;
    impl ListConfig for TestConfigSized {
        type Item = u8;
        fn get_usersize(item: &u8) -> usize {
            *item as usize
        }

        fn split_item(item: &u8, at: usize) -> (u8, u8) {
            (at as u8, item - at as u8)
        }
    }

    fn check<'a, C: ListConfig>(list: &SkipList<C>, expected: &'a [C::Item])
        where C::Item: PartialEq + Debug + Copy
    {
        list.print();
        list.check();
        assert!(list.eq_list(expected));
        
        let vec: Vec<C::Item> = list.into();
        assert_eq!(vec, expected);
        assert_eq!(list.len_items(), expected.len());
        assert_eq!(list.get_userlen(), C::userlen_of_slice(expected));

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
        let list = SkipList::<TestConfigFlat>::new();
        assert!(list.eq_list(&[]));
        assert!(!list.eq_list(&[1]));
        check(&list, &[]);
        
        let list = SkipList::<TestConfigFlat>::new_from_slice(&[1,2,3,4]);
        assert!(list.eq_list(&[1,2,3,4]));
        assert!(!list.eq_list(&[1,2,3,5]));
        assert!(!list.eq_list(&[1,2,3]));
        check(&list, &[1,2,3,4]);
    }

    #[test]
    fn simple_edits() {
        let mut list = SkipList::<TestConfigFlat>::new_from_slice(&[1,2,3,4]);
        check(&list, &[1,2,3,4]);
        
        list.del_at(1, 2);
        check(&list, &[1,4]);
        
        list.replace_at_slice(1, 1, &[5,6,7]);
        check(&list, &[1,5,6,7]);
    }
    
    #[test]
    fn empty_list_has_no_contents() {
        let mut list = SkipList::<TestConfigFlat>::new();
        check(&list, &[]);

        list.insert_at_slice(0, &[]);
        check(&list, &[]);
    }

    #[test]
    fn insert_at_location() {
        let mut list = SkipList::<TestConfigFlat>::new();

        list.insert_at_slice(0, &[1,1,1]);
        check(&list, &[1,1,1]);

        list.insert_at_slice(0, &[2,2,2]);
        check(&list, &[2,2,2,1,1,1]);

        list.insert_at_slice(6, &[3,3,3]);
        check(&list, &[2,2,2,1,1,1,3,3,3]);

        list.insert_at_slice(5, &[4,4,4]);
        check(&list, &[2,2,2,1,1,4,4,4,1,3,3,3]);
    }

    #[test]
    fn insert_between() {
        let mut list = SkipList::<TestConfigSized>::new_from_slice(&[5,2]);
        
        list.insert_at_slice(1, &[10]);
        check(&list, &[1,10,4,2]);
    }

    #[test]
    fn del_at_location() {
        let mut list = SkipList::<TestConfigFlat>::new_from_slice(&[0,1,2,3,4,5,6,7,8]);

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

        let mut list = SkipList::<TestConfigFlat>::new_from_slice(content.as_slice());
        check(&list, content.as_slice());

        // Delete everything but the first and last characters
        list.del_at(1, len - 2);
        check(&list, &[content[0], content[len-1]]);
    }

    #[test]
    fn nonuniform_edits() {
        let mut list = SkipList::<TestConfigSized>::new();
        check(&list, &[]);
        
        list.insert_at_slice(0, &[2,1]);
        check(&list, &[2,1]);

        list.insert_at_slice(2, &[0,0]);
        check(&list, &[2,0,0,1]);
        
        list.insert_at_slice(3, &[5]);
        check(&list, &[2,0,0,1,5]);
        
        list.del_at(3, 1);
        check(&list, &[2,0,0,1]);

        list.insert_at_slice(2, &[5,5]); // Inserted items go as far left as possible.
        check(&list, &[2,5,5,0,0,1]);

        list.del_at(12, 2);
        check(&list, &[2,5,5,1]);
    }

    #[test]
    fn modify_item() {
        let mut list = SkipList::<TestConfigSized>::new_from_slice(&[5,4,3,2,1]);
        list.modify_item_at(5, |item, offset| {
            assert_eq!(offset, 0);
            *item = 10;
        });
        check(&list, &[5,10,3,2,1]);

        list.modify_item_at(17, |item, offset| {
            assert_eq!(offset, 2); // And check a non-zero offset.
            *item = 1;
        });
        check(&list, &[5,10,1,2,1]);
    }


    // Trashy non-performant implementation of the API for randomized testing.
    fn vec_find_userpos<C: ListConfig>(list: &Vec<C::Item>, target_userpos: usize) -> usize {
        let mut item_pos = 0;
        let mut userpos = 0;
        while userpos != target_userpos {
            assert!(item_pos < list.len(), "Trying to insert past the end");
            let usersize = C::get_usersize(&list[item_pos]);
            userpos += usersize;
            assert!(userpos <= target_userpos, "Cannot split items");
            item_pos += 1;
        }
        item_pos
    }

    fn vec_insert_at<C: ListConfig>(list: &mut Vec<C::Item>, target_userpos: usize, content: &[C::Item]) where C::Item: Copy {
        let mut item_pos = vec_find_userpos::<C>(list, target_userpos);
        
        for item in content {
            // This is O(n^2) because of the moves, but this is testing code and
            // its fine. The old code was more complex to make this fast, but I
            // thats probably overkill here.
            list.insert(item_pos, *item);
            item_pos += 1;
        }
    }

    fn vec_delete_at<C: ListConfig>(list: &mut Vec<C::Item>, target_userpos: usize, num_items: usize) {
        let item_pos = vec_find_userpos::<C>(list, target_userpos);

        list.drain(item_pos .. item_pos+num_items);
    }

    fn vec_replace<C: ListConfig>(list: &mut Vec<C::Item>, target_userpos: usize, removed_items: usize, inserted_content: &[C::Item]) where C::Item: Copy {
        vec_delete_at::<C>(list, target_userpos, removed_items);
        vec_insert_at::<C>(list, target_userpos, inserted_content);
    }


    use self::rand::{SeedableRng, rngs::SmallRng};

    fn gen_random_data<C: ListConfig>(max_len: usize, rng: &mut SmallRng, gen_item: fn(r: &mut SmallRng) -> C::Item) -> Vec::<C::Item> {
        let mut content = Vec::<C::Item>::new();
        // Sometimes generate strings longer than a single node to stress everything.
        for _ in 0..rng.gen_range(0, max_len) { // This should bias toward smaller inserts.
            content.push(gen_item(rng));
        }

        content
    }

    fn random_edits<C: ListConfig>(gen_item: fn(r: &mut SmallRng) -> C::Item) where C::Item: PartialEq + Debug + Copy {
        let mut list = SkipList::<C>::new();
        let mut vec = Vec::<<C as ListConfig>::Item>::new();

        let mut rng = SmallRng::seed_from_u64(321);

        let target_min = 800;
        let target_max = 1200;
        let max_chunk_size = 50;

        for i in 0..1000 {
            let itemlen = vec.len();
            let userlen = C::userlen_of_slice(vec.as_slice());
            // let len = vec.chars().count();

            println!("i {}: items: {} / user: {}", i, itemlen, userlen);
            
            if itemlen == 0 || (itemlen < target_min && rng.gen::<f32>() < 0.35) {
                // Insert.
                let itempos = rng.gen_range(0, itemlen+1);
                let userpos = C::userlen_of_slice(&vec[0..itempos]);
                if itemlen > 0 { assert!(userlen > 0); }
                
                let content = gen_random_data::<C>(max_chunk_size, &mut rng, gen_item);

                println!("insert {} content", content.len());
                list.insert_at_slice(userpos, content.as_slice());
                vec_insert_at::<C>(&mut vec, userpos, content.as_slice());

                check(&list, vec.as_slice());
            } else if itemlen > target_max || rng.gen::<f32>() < 0.5 {
                // Delete
                let itempos = rng.gen_range(0, itemlen+1); // Sometimes delete nothing at the end.
                let userpos = C::userlen_of_slice(&vec[0..itempos]);

                // Again some biasing here would be good.
                let num_deleted_items = std::cmp::min(rng.gen_range(0, max_chunk_size), vec.len() - itempos);

                println!("delete {} items", num_deleted_items);
                list.del_at(userpos, num_deleted_items);
                vec_delete_at::<C>(&mut vec, userpos, num_deleted_items);

                check(&list, vec.as_slice());
            } else {
                // Replace something!
                let itempos = rng.gen_range(0, itemlen+1);
                let userpos = C::userlen_of_slice(&vec[0..itempos]);

                let num_deleted_items = std::cmp::min(rng.gen_range(0, max_chunk_size), vec.len() - itempos);
                let ins_content = gen_random_data::<C>(max_chunk_size, &mut rng, gen_item);

                println!("replace {} with {} items", num_deleted_items, ins_content.len());
                list.replace_at_slice(userpos, num_deleted_items, ins_content.as_slice());
                vec_replace::<C>(&mut vec, userpos, num_deleted_items, ins_content.as_slice());

                check(&list, vec.as_slice());
            }
        }
    }

    #[test]
    fn random_edits_flat() {
        random_edits::<TestConfigFlat>(|rng| rng.gen_range(0, 10));
    }

    #[test]
    fn random_edits_nonuniform() {
        random_edits::<TestConfigSized>(|rng| rng.gen_range(0, 10));
    }


    // use std::marker::PhantomData;
    struct TestConfigDrop();
    impl<'a> ListConfig for &'a TestConfigDrop {
        type Item = TDItem<'a>;
    }

    #[test]
    fn inserted_contents_dropped() {
        let td = TestDrop::new();
        let mut list = SkipList::<&TestConfigDrop>::new();
        
        let (id, item) = td.new_item();
        list.insert_at(0, iter::once(item));
        
        drop(list);
        td.assert_drop(id);
    }

    #[test]
    fn replaced_contents_dropped() {
        let td = TestDrop::new();
        let mut list = SkipList::<&TestConfigDrop>::new();
        
        let (id1, item1) = td.new_item();
        list.insert_at(0, iter::once(item1));

        let (id2, item2) = td.new_item();
        list.replace_at(0, 1, iter::once(item2));
        td.assert_drop(id1);

        drop(list);
        td.assert_drop(id2);
    }

    #[test]
    fn deleted_contents_dropped() {
        let td = TestDrop::new();
        let mut list = SkipList::<&TestConfigDrop>::new();
        
        let (id, item) = td.new_item();
        list.insert_at(0, iter::once(item));

        list.del_at(0, 1);
        td.assert_drop(id);
    }
}
