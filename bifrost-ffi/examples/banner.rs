//! Example: print the vendored Bifrost FFI summary at startup.
//!
//! Run with: `cargo run --release --example banner` from the bifrost-ffi dir.

fn main() {
    let s = argis_bifrost_ffi::summary();
    println!("argis-bifrost-ffi: {s}");
}
