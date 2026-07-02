package api

import (
	"bytes"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
)

// FuzzDecodeAndValidateJSON exercises the security-critical body parser
// from PR #176 (audit finding L22 — no fuzz coverage). The fuzzer feeds
// arbitrary bytes into decodeAndValidateJSON and asserts:
//
//  1. The parser never panics, regardless of input.
//  2. The parser never returns a nil error AND leaves the destination
//     struct populated with garbage that violates a `validate:"required"`
//     constraint (the validation step must run after every successful
//     JSON decode).
//  3. The parser never returns success for inputs larger than the
//     configured maxBytes (no silent truncation bug).
//
// Native Go fuzzing (testing.F) — no external dependency.
func FuzzDecodeAndValidateJSON(f *testing.F) {
	// Seed corpus: representative valid + malformed payloads so the
	// fuzzer has a head start on coverage.
	f.Add([]byte(`{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}`))
	f.Add([]byte(`{"model":"","messages":[]}`))
	f.Add([]byte(`not json`))
	f.Add([]byte(``))
	f.Add([]byte(`{"model":"x","messages":[{"role":"unknown","content":"y"}]}`))
	f.Add([]byte(`{"model":"x","messages":[{"role":"user","content":"y"}],"temperature":99}`))
	f.Add([]byte(`{"model":"x","messages":[{"role":"user","content":"y"}],"max_tokens":-1}`))
	f.Add([]byte(`{`))
	f.Add([]byte(`}`))
	f.Add([]byte(`{"messages":[{"role":"user","content":"y"}]`)) // trailing
	f.Add([]byte(`{"extra":"field","model":"x","messages":[]}`))

	const maxBytes int64 = 4096

	f.Fuzz(func(t *testing.T, data []byte) {
		// Build a fresh request per iteration so httptest state can't
		// bleed between runs.
		req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", bytes.NewReader(data))
		req.Header.Set("Content-Type", "application/json")

		var dst ChatCompletionRequest
		err := decodeAndValidateJSON(req, &dst, maxBytes)

		// Property 1: must never panic. (If we reach this line, no panic
		// occurred — fuzz failures show up as `FAIL` with a stack trace.)

		// Property 2: if err is nil, the validated struct must satisfy the
		// `required` constraints. This catches a class of bug where the
		// validator silently stops running.
		if err == nil {
			if dst.Model == "" {
				t.Fatalf("decodeAndValidateJSON returned no error but Model is empty: %+v", dst)
			}
			if len(dst.Messages) == 0 {
				t.Fatalf("decodeAndValidateJSON returned no error but Messages is empty: %+v", dst)
			}
			for i, m := range dst.Messages {
				if m.Role == "" || m.Content == "" {
					t.Fatalf("message %d missing required fields after validation: %+v", i, dst.Messages)
				}
			}
			if dst.Temperature < 0 || dst.Temperature > 2 {
				t.Fatalf("temperature %f outside [0,2] after validation: %+v", dst.Temperature, dst)
			}
			if dst.MaxTokens < 0 {
				t.Fatalf("negative MaxTokens after validation: %+v", dst)
			}
		}

		// Property 3: maxBytes enforcement. If the body was larger than
		// maxBytes, the call must have failed with a bodyError. We can't
		// assert the exact error (json.Decoder formats vary by Go
		// version), but we MUST get an error — otherwise oversized
		// payloads would slip past the protection.
		if len(data) > int(maxBytes) {
			if err == nil {
				t.Fatalf("expected body-size rejection for %d bytes (>maxBytes=%d), got nil error", len(data), maxBytes)
			}
		}
	})
}

// FuzzContentTypeJSON feeds arbitrary method + content-type combinations
// into the ContentTypeJSON middleware and asserts the policy holds:
// mutating methods without application/json must yield 415, while GET
// passes through regardless.
func FuzzContentTypeJSON(f *testing.F) {
	f.Add("POST", "application/json")
	f.Add("POST", "text/plain")
	f.Add("POST", "")
	f.Add("PUT", "application/xml")
	f.Add("PATCH", "application/json; charset=utf-8")
	f.Add("GET", "")
	f.Add("DELETE", "text/html")
	f.Add("HEAD", "application/json")

	mutating := map[string]bool{
		http.MethodPost:  true,
		http.MethodPut:   true,
		http.MethodPatch: true,
	}

	next := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
	})

	f.Fuzz(func(t *testing.T, method, contentType string) {
		req := httptest.NewRequest(method, "/anything", io.EOF)
		if contentType != "" {
			req.Header.Set("Content-Type", contentType)
		}
		rec := httptest.NewRecorder()
		ContentTypeJSON(next).ServeHTTP(rec, req)

		if mutating[method] && contentType != "application/json" {
			if rec.Code != http.StatusUnsupportedMediaType {
				t.Fatalf("%s with Content-Type=%q: expected 415, got %d", method, contentType, rec.Code)
			}
		} else {
			if rec.Code != http.StatusOK {
				t.Fatalf("%s with Content-Type=%q: expected 200 passthrough, got %d", method, contentType, rec.Code)
			}
		}
	})
}
