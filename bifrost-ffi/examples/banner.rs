//! Example: print the vendored Bifrost FFI summary at startup.
//!
//! Run with: `cargo run --release --example banner` from the bifrost-ffi dir.

fn main() {
    let v = argis_bifrost_ffi::version();
    let n = argis_bifrost_ffi::provider_count();
    let names = argis_bifrost_ffi::provider_names();
    println!("argis-bifrost-ffi: {v} ({n} providers)");
    for name in names {
        println!("  - {name}");
    }
}
