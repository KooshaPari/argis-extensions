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
	"encoding/json"
	"fmt"

	"github.com/maximhq/bifrost/core/schemas"
)

// #include <stdint.h>
import "C"

//export bifrost_version
func bifrost_version() *C.char {
	return C.CString("bifrost-ffi v0.3.0 (vendored from maximhq/bifrost/core v1.2.30)")
}

//export bifrost_provider_count
func bifrost_provider_count() C.int {
	return C.int(4)
}

//export bifrost_provider_name
func bifrost_provider_name(index C.int) *C.char {
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

//export bifrost_chat_completion
func bifrost_chat_completion(model *C.char, prompt *C.char) *C.char {
	// Real chat completion using the upstream schemas.CompletionRequest
	// and schemas.CompletionResponse types. This is the test of the wrap
	// pattern's most production-relevant path: the actual LLM gateway
	// request/response cycle (modulo the actual HTTP call to a real
	// provider, which is mocked here for hermetic testing).
	modelStr := C.GoString(model)
	promptStr := C.GoString(prompt)

	// Build a real CompletionRequest using the upstream type. The content
	// array is one user message with the prompt text.
	req := schemas.CompletionRequest{
		
		Model:    modelStr,
		Messages: []schemas.Message{
			{
				Role:    "user",
				Content: promptStr,
			},
		},
	}

	// A real CompletionResponse matching the upstream schema. The mock
	// just echoes the prompt back; a real provider would call the API.
	resp := schemas.CompletionResponse{
		ID:      "chatcmpl-argis-monitor-demo",
		Content: "echo: " + promptStr,
		Model:   modelStr,
		Usage: schemas.Usage{
			PromptTokens:     len(promptStr),
			CompletionTokens: len(promptStr) + 6,
			TotalTokens:      2*len(promptStr) + 6,
		},
	}

	// Wrap in a BifrostResponse to match the real API surface.
	br := schemas.BifrostResponse{
		CompletionResponse: &resp,
	}

	// Serialize the FULL request and the FULL response as JSON so the
	// Rust side can inspect both. A real integration would parse these
	// back into the matching Rust types.
	raw, err := json.MarshalIndent(struct {
		Request  schemas.CompletionRequest  `json:"request"`
		Response schemas.BifrostResponse     `json:"response"`
	}{req, br}, "", "  ")
	if err != nil {
		return C.CString(fmt.Sprintf("error: %v", err))
	}
	return C.CString(string(raw))
}

func main() {}
