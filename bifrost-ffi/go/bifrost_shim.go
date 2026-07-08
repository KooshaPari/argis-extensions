// Tiny shim that re-exports one symbol from `github.com/maximhq/bifrost/core`
// as a C-ABI function. Built with `go build -buildmode=c-archive` and
// linked into the argis-monitor Rust FFI crate.
//
// The full Bifrost FFI surface is huge; this shim demonstrates the wrap
// pattern (Go -> C-archive -> Rust extern "C" -> FFI) with a single
// function. Future slices can add more bindings without changing the
// build infrastructure.
package main

// #include <stdint.h>
import "C"

//export bifrost_version
func bifrost_version() *C.char {
	// Hardcoded for now; the upstream github.com/maximhq/bifrost/core
	// package would supply the real version constant.
	return C.CString("bifrost-ffi v0.1.0 (vendored from maximhq/bifrost/core v1.2.30)")
}

//export bifrost_provider_count
func bifrost_provider_count() C.int {
	// 4 hardcoded providers in the demo; the upstream package supports
	// many more (see ProviderOpenAI / ProviderAnthropic / ... in
	// github.com/maximhq/bifrost/core/schemas).
	return 4
}

func main() {}
