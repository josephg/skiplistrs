# Fancy skiplist in Rust

> Status: Experimental. Lots of API churn still happening.

This is a high performance skiplist implementation packed with useful features. Skip lists are super fun structures which can be used in a similar way to a B-Tree, but with an arguably simpler implementation. Unlike a vec, you can efficiently insert and delete items anywhere in the list in /log(n)/ time. This list handles millions of edits per second even for large lists.

This skip list implementation has the following extra fancy features:

- It supports defining a custom length for list items. Your custom length function is used for item positions, so when you locate an item, you use the user size sum as the index.
- You can use a secondary index to refer to items in the skiplist. The secondary index can reference an item, and despite the item moving around due to inserts and deletes, your marker can still be used to:
  - Find and edit the item (or adjacent items) in the list
  - Look up the position of the item your marker points to
- It has a lightweight transaction cursor, which can be used for complex edits

Experimental features:

- You can have items be automatically split when inserting. So, if you want to insert in the middle of an item, you can define a split function for your type and then insert an item right in the middle of another item. Your item will be split automatically.
- /Planned, not implemented/: Automatic merging, so if an item is inserted after another item and both items can be merged together, the item will be extended instead.



## History

This code was originally [written in C](https://github.com/josephg/librope) as a high performance unicode rope (supporting efficient inserts and removes at arbitrary positions in strings). It was then [ported to rust](https://github.com/josephg/rustrope) and finally adapted here for custom data structures. I recommend not using this implementation for strings - a specialized rope implementation will work better.

This is still a work in progress. There may be bugs. For usage examples, see [tests/test.rs](tests/test.rs).


# LICENSE

Licensed under the ISC license:

Copyright 2018 Joseph Gentle

Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.