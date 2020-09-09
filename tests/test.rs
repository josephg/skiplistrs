// These tests are also adapted from the C code tests here:
// https://github.com/josephg/librope/blob/master/test/tests.c

#[cfg(test)]
mod test {

    extern crate skiplistrs;
    use self::skiplistrs::*;

    extern crate rand;
    use self::rand::Rng;

    use std::cmp::min;

    use std::fmt::Debug;

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
    }

    fn check<'a, C: ListConfig>(list: &SkipList<C>, expected: &'a [C::Item])
        where C::Item: PartialEq + Debug
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
        
        list.replace_at(1, 1, &[5,6,7]);
        check(&list, &[1,5,6,7]);
    }
    
    #[test]
    fn empty_list_has_no_contents() {
        let mut list = SkipList::<TestConfigFlat>::new();
        check(&list, &[]);

        list.insert_at(0, &[]);
        check(&list, &[]);
    }

    #[test]
    fn insert_at_location() {
        let mut list = SkipList::<TestConfigFlat>::new();

        list.insert_at(0, &[1,1,1]);
        check(&list, &[1,1,1]);

        list.insert_at(0, &[2,2,2]);
        check(&list, &[2,2,2,1,1,1]);

        list.insert_at(6, &[3,3,3]);
        check(&list, &[2,2,2,1,1,1,3,3,3]);

        list.insert_at(5, &[4,4,4]);
        check(&list, &[2,2,2,1,1,4,4,4,1,3,3,3]);
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
        
        list.insert_at(0, &[2,1]);
        check(&list, &[2,1]);

        list.insert_at(2, &[0,0]);
        check(&list, &[2,0,0,1]);
        
        list.insert_at(3, &[5]);
        check(&list, &[2,0,0,1,5]);
        
        list.del_at(3, 1);
        check(&list, &[2,0,0,1]);

        list.insert_at(2, &[5,5]); // Inserted items go as far left as possible.
        check(&list, &[2,5,5,0,0,1]);

        list.del_at(12, 2);
        check(&list, &[2,5,5,1]);
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

    fn vec_insert_at<C: ListConfig>(list: &mut Vec<C::Item>, target_userpos: usize, content: &[C::Item]) {
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

    fn vec_replace<C: ListConfig>(list: &mut Vec<C::Item>, target_userpos: usize, removed_items: usize, inserted_content: &[C::Item]) {
        let item_pos = vec_find_userpos::<C>(list, target_userpos);

        vec_delete_at::<C>(list, target_userpos, removed_items);
        vec_insert_at::<C>(list, target_userpos, inserted_content);
    }


    use self::rand::{SeedableRng, rngs::SmallRng};

    fn random_edits<C: ListConfig>(gen_item: fn(r: &mut SmallRng) -> C::Item) where C::Item: PartialEq + Debug {
        let mut list = SkipList::<C>::new();
        let mut vec = Vec::<<C as ListConfig>::Item>::new();

        let mut rng = SmallRng::seed_from_u64(321);

        for _ in 0..1000 {
            check(&list, vec.as_slice());

            let itemlen = vec.len();
            let userlen = C::userlen_of_slice(vec.as_slice());
            // let len = vec.chars().count();

            // println!("i {}: {}", i, len);
            
            if itemlen == 0 || (itemlen < 1000 && rng.gen::<f32>() < 0.5) {
                // Insert.
                let ins_itempos = rng.gen_range(0, itemlen+1);
                let ins_userpos = C::userlen_of_slice(&vec[0..ins_itempos]);
                if itemlen > 0 { assert!(userlen > 0); }
                
                let mut content = Vec::<C::Item>::new();
                // Sometimes generate strings longer than a single node to stress everything.
                for _ in 0..rng.gen_range(0, 500) { // This should bias toward smaller inserts.
                    content.push(gen_item(&mut rng));
                }

                list.insert_at(ins_userpos, content.as_slice());
                vec_insert_at::<C>(&mut vec, ins_userpos, content.as_slice());
            } else {
                // Delete
                let del_itempos = rng.gen_range(0, itemlen+1); // Sometimes delete nothing at the end.
                let del_userpos = C::userlen_of_slice(&vec[0..del_itempos]);

                // Again some biasing here would be good.
                let num_deleted_items = if vec.len() == del_itempos { 0 }
                else { rng.gen_range(0, std::cmp::min(30, vec.len() - del_itempos)) };

                list.del_at(del_userpos, num_deleted_items);
                vec_delete_at::<C>(&mut vec, del_userpos, num_deleted_items);
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
}
