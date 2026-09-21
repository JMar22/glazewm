# shell-util (vendored)

A copy of the `shell-util` 0.0.0 crate from crates.io, published by Glzr
Software from `glzr-io/zebar` at commit
`1a5c9125e8e79e8dd4a133bdc919ad7282c12b47` (`crates/shell-util`), under
the MIT license declared in its manifest.

It is vendored so this fork builds on stable Rust. The published crate is
nightly-only: it enables `#![feature(slice_internals)]` to call
`core::slice::memchr`. The only change is that `find_delimiter` in
`src/stdout_reader.rs` searches with `Iterator::position` instead, which
returns the same first match. Everything else is the published source.

The root `Cargo.toml` substitutes this copy for the crates.io one through
`[patch.crates-io]`. Drop the directory and that entry once an upstream
release builds on stable.
