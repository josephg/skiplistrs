# Skiplist in Rust

> Status: Experimental

This is a high performance implementation of a skiplist in rust. Skip lists are super fun structures which can be used as you would a B-Tree. Their performance is similar, and their implementation is arguably slightly more simple.

This skip list is designed for items with variable sizes (eg run-length encoded nodes). When inserting or removing items, positions can be passed using the custom units.

This code was originally [written in C](https://github.com/josephg/librope) as a high performance unicode rope (supporting efficient inserts and removes at arbitrary positions in strings). It was then [ported to rust](https://github.com/josephg/rustrope) and finally adapted here for custom data structures. I recommend not using this implementation for ropes.

This is still a work in progress. There may be bugs. For usage examples, see [tests/test.rs](tests/test.rs).


# LICENSE

Licensed under the ISC license:

Copyright 2018 Joseph Gentle

Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.