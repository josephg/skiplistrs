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
        
        list.del_at(1, 2); // DeleTestConfigFlat
        check(&list, &[1,4]);
        // list.print();
        
        list.replace_at(1, 1, &[5,6,7]); // List should now be 1, 1, 2, 3, 4.
        check(&list, &[1,5,6,7]);
        // list.print();
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

    // #[test]
    // fn new_string_has_content() {
    //     let r = SkipList::new_from_str("hi there");
    //     check(&r, "hi there");

    //     let mut r = SkipList::new_from_str("κόσμε");
    //     check(&r, "κόσμε");
    //     r.insert_at(2, "𝕐𝕆😘");
    //     check(&r, "κό𝕐𝕆😘σμε");
    // }

    // #[test]
    // fn del_at_location() {
    //     let mut r = SkipList::new_from_str("012345678");

    //     r.del_at(8, 1);
    //     check(&r, "01234567");
        
    //     r.del_at(0, 1);
    //     check(&r, "1234567");
        
    //     r.del_at(5, 1);
    //     check(&r, "123457");
        
    //     r.del_at(5, 1);
    //     check(&r, "12345");
        
    //     r.del_at(0, 5);
    //     check(&r, "");
    // }

    // #[test]
    // fn del_past_end_of_string() {
    //     let mut r = SkipList::new();

    //     r.del_at(0, 100);
    //     check(&r, "");

    //     r.insert_at(0, "hi there");
    //     r.del_at(3, 10);
    //     check(&r, "hi ");
    // }

    // #[test]
    // fn really_long_ascii_string() {
    //     let len = 2000;
    //     let s = random_ascii_string(len);

    //     let mut r = SkipList::new_from_str(s.as_str());
    //     check(&r, s.as_str());

    //     // Delete everything but the first and last characters
    //     r.del_at(1, len - 2);
    //     let expect = format!("{}{}", s.as_bytes()[0] as char, s.as_bytes()[len-1] as char);
    //     check(&r, expect.as_str());
    // }


    // use std::ptr;

    // fn string_insert_at(s: &mut String, char_pos: usize, contents: &str) {
    //     // If you try to write past the end of the string for now I'll just write at the end.
    //     // Panicing might be a better policy.
    //     let byte_pos = s.char_indices().skip(char_pos).next()
    //         .map(|(p, _)| p).unwrap_or(s.len());
        
    //     let old_len = s.len();
    //     let new_bytes = contents.len();

    //     // This didn't work because it didn't change the string's length
    //     //s.reserve(new_bytes);

    //     // This is sort of ugly but its fine.
    //     for _ in 0..new_bytes { s.push('\0'); }

    //     //println!("new bytes {} {} {}", new_bytes, byte_pos, s.len() - byte_pos);
    //     unsafe {
    //         let bytes = s.as_mut_vec().as_mut_ptr();
    //         ptr::copy(
    //             bytes.offset(byte_pos as isize),
    //             bytes.offset((byte_pos + new_bytes) as isize),
    //             old_len - byte_pos
    //         );
    //         ptr::copy_nonoverlapping(
    //             contents.as_ptr(),
    //             bytes.offset(byte_pos as isize),
    //             new_bytes
    //         );
    //     }
    // }

    // fn string_del_at(s: &mut String, pos: usize, length: usize) {
    //     let byte_range = {
    //         let mut iter = s.char_indices().map(|(p, _)| p).skip(pos).peekable();

    //         let start = iter.peek().map_or_else(|| s.len(), |&p| p);
    //         let mut iter = iter.skip(length).peekable();
    //         let end = iter.peek().map_or_else(|| s.len(), |&p| p);

    //         start..end
    //     };

    //     s.drain(byte_range);
    // }



    // #[test]
    // fn random_edits() {
    //     let mut r = SkipList::new();
    //     let mut s = String::new();
        
    //     let mut rng = rand::thread_rng();

    //     for _ in 0..1000 {
    //         check(&r, s.as_str());

    //         let len = s.chars().count();

    //         // println!("i {}: {}", i, len);
            
    //         if len == 0 || (len < 1000 && rng.gen::<f32>() < 0.5) {
    //             // Insert.
    //             let pos = rng.gen_range(0, len+1);
    //             // Sometimes generate strings longer than a single node to stress everything.
    //             let text = random_unicode_string(rng.gen_range(0, 1000));
    //             r.insert_at(pos, text.as_str());
    //             string_insert_at(&mut s, pos, text.as_str());
    //         } else {
    //             // Delete
    //             let pos = rng.gen_range(0, len);
    //             let dlen = min(rng.gen_range(0, 10), len - pos);

    //             r.del_at(pos, dlen);
    //             string_del_at(&mut s, pos, dlen);
    //         }
    //     }
    // }
}
