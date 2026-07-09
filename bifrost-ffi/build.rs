// Build script: ensures the Go C-archive exists, then tells rustc where
// to find it. The vendored Go shim in `go/` is the source of truth; no
// network fetches are required.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set");
    let go_dir = Path::new(&manifest_dir).join("go");
    let archive = go_dir.join("libbifrost_shim.a");
    let header = go_dir.join("libbifrost_shim.h");

    // Rebuild the C-archive if missing or older than its sources.
    let needs_rebuild = !archive.exists()
        || !header.exists()
        || source_mtime(&go_dir.join("bifrost_shim.go")) > archive_mtime(&archive)
        || source_mtime(&go_dir.join("go.mod")) > archive_mtime(&archive);

    if needs_rebuild {
        eprintln!(
            "argis-bifrost-ffi: building Go C-archive ({} -> {})",
            go_dir.display(),
            archive.display()
        );
        let status = Command::new("go")
            .arg("build")
            .arg("-buildmode=c-archive")
            .arg("-o")
            .arg(&archive)
            .arg("./")
            .current_dir(&go_dir)
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => panic!(
                "argis-bifrost-ffi: `go build` exited with status {s}; \
                 ensure `go` (>=1.21) is installed and the go/ shim compiles"
            ),
            Err(e) => panic!(
                "argis-bifrost-ffi: failed to spawn `go`: {e}; \
                 install Go >= 1.21 to build the FFI shim"
            ),
        }
    }

    println!("cargo:rustc-link-search=native={}", go_dir.display());
    println!("cargo:rustc-link-lib=static=bifrost_shim");
    // Re-run if the archive or header is updated.
    println!("cargo:rerun-if-changed=go/bifrost_shim.go");
    println!("cargo:rerun-if-changed=go/go.mod");
    println!("cargo:rerun-if-changed=go/libbifrost_shim.a");
    println!("cargo:rerun-if-changed=go/libbifrost_shim.h");

    // Belt-and-braces: also expose the Go directory to rustc's include
    // path so #[cfg(feature = "vendored")] can include the header if needed.
    let _ = PathBuf::from(&manifest_dir);
}

fn archive_mtime(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

fn source_mtime(p: &Path) -> std::time::SystemTime {
    archive_mtime(p)
}
