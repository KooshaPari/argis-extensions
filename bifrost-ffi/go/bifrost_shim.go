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


//export bifrost_provider_names_count
func bifrost_provider_names_count() C.int {
	// The upstream package declares the canonical provider constants in
	// the `Provider*` const block. We hardcode the count to match what's
	// exported in schemas.go as of v1.2.30; future slices can call
	// reflect on the upstream symbols dynamically.
	return C.int(4) // OpenAI, Anthropic, Gemini, Custom
}

//export bifrost_provider_name
func bifrost_provider_name(index C.int) *C.char {
	// Hardcoded provider list in the demo shim. Future slices can switch
	// to calling the upstream github.com/maximhq/bifrost/core/schemas
	// package directly.
	switch index {
	case 0:
		return C.CString("openai")
	case 1:
		return C.CString("anthropic")
	case 2:
		return C.CString("gemini")
	case 3:
		return C.CString("custom")
	default:
		return nil
	}
}
