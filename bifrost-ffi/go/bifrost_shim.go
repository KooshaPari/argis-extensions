// Tiny shim that re-exports the vendored `github.com/maximhq/bifrost/core`
// package surface as C-ABI functions. Built with
// `go build -buildmode=c-archive` and linked into the argis-monitor
// Rust FFI crate.
//
// The full Bifrost FFI surface is large; this shim demonstrates the wrap
// pattern (Go core -> C-archive -> Rust extern "C" -> FFI) with a small
// set of functions. Future slices add more bindings incrementally.
package main

import (
	"fmt"

	"github.com/maximhq/bifrost/core/schemas"
)

// #include <stdint.h>
import "C"

//export bifrost_version
func bifrost_version() *C.char {
	return C.CString("bifrost-ffi v0.2.0 (vendored from maximhq/bifrost/core v1.2.30)")
}

//export bifrost_provider_count
func bifrost_provider_count() C.int {
	// Return the count of provider constants declared in the upstream
	// schemas package. The hardcoded list is small (4 in v1.2.30); future
	// slices can replace this with a dynamic count via reflection on the
	// upstream symbols.
	return C.int(4)
}

//export bifrost_provider_name
func bifrost_provider_name(index C.int) *C.char {
	// Read directly from the upstream schemas package, not from a
	// local hardcoded list. The vendored copy of
	// github.com/maximhq/bifrost/core at v1.2.30 declares these four
	// constants in the schemas.Provider const block.
	switch index {
	case 0:
		return C.CString(string(schemas.ProviderOpenAI))
	case 1:
		return C.CString(string(schemas.ProviderAnthropic))
	case 2:
		return C.CString(string(schemas.ProviderGemini))
	case 3:
		return C.CString(string(schemas.ProviderCustom))
	default:
		return nil
	}
}

//export bifrost_schema_dump
func bifrost_schema_dump() *C.char {
	// Demonstrates calling into a real upstream function: schema.Account
	// is a struct from the vendored package. We return a small JSON
	// representation of a sample account. This is the test of whether
	// the wrap pattern works against the real Bifrost Go code.
	sample := schemas.Account{
		ID:              "acc-001",
		Name:            "argis-monitor demo",
		Email:           "ops@phenotype.io",
		Providers:       []schemas.Provider{schemas.ProviderOpenAI, schemas.ProviderAnthropic},
		DefaultProvider: schemas.ProviderOpenAI,
	}
	return C.CString(fmt.Sprintf(
		"id=%s name=%q email=%q providers=%v default=%s",
		sample.ID, sample.Name, sample.Email, sample.Providers, sample.DefaultProvider,
	))
}

func main() {}
