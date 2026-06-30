package api

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestDecodeAndValidateJSON_ValidChatRequest(t *testing.T) {
	body := `{"model":"gpt-4","messages":[{"role":"user","content":"hello"}]}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst ChatCompletionRequest
	if err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if dst.Model != "gpt-4" {
		t.Errorf("expected model gpt-4, got %s", dst.Model)
	}
	if len(dst.Messages) != 1 || dst.Messages[0].Content != "hello" {
		t.Errorf("unexpected messages: %+v", dst.Messages)
	}
}

func TestDecodeAndValidateJSON_MissingModel(t *testing.T) {
	body := `{"messages":[{"role":"user","content":"hi"}]}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst ChatCompletionRequest
	err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize)
	if err == nil {
		t.Fatal("expected validation error for missing model")
	}
	if be, ok := err.(*bodyError); ok {
		if be.Status != http.StatusUnprocessableEntity {
			t.Errorf("expected status 422, got %d", be.Status)
		}
	} else {
		t.Errorf("expected *bodyError, got %T: %v", err, err)
	}
}

func TestDecodeAndValidateJSON_EmptyMessages(t *testing.T) {
	body := `{"model":"gpt-4","messages":[]}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst ChatCompletionRequest
	err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize)
	if err == nil {
		t.Fatal("expected validation error for empty messages")
	}
}

func TestDecodeAndValidateJSON_InvalidRole(t *testing.T) {
	body := `{"model":"gpt-4","messages":[{"role":"superadmin","content":"hi"}]}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst ChatCompletionRequest
	err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize)
	if err == nil {
		t.Fatal("expected validation error for invalid role")
	}
}

func TestDecodeAndValidateJSON_BodyTooLarge(t *testing.T) {
	// Build a body that exceeds a very small limit.
	body := strings.Repeat("a", 200)
	body = `{"model":"x","messages":[{"role":"user","content":"` + body + `"}]}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst ChatCompletionRequest
	err := decodeAndValidateJSON(req, &dst, 100) // 100 byte limit
	if err == nil {
		t.Fatal("expected error for oversized body")
	}
}

func TestDecodeAndValidateJSON_TrailingData(t *testing.T) {
	body := `{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}extra`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst ChatCompletionRequest
	err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize)
	if err == nil {
		t.Fatal("expected error for trailing data")
	}
}

func TestDecodeAndValidateJSON_UnknownFields(t *testing.T) {
	body := `{"model":"gpt-4","messages":[{"role":"user","content":"hi"}],"unknown_field":123}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst ChatCompletionRequest
	err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize)
	if err == nil {
		t.Fatal("expected error for unknown fields")
	}
}

func TestDecodeAndValidateJSON_ValidCompletion(t *testing.T) {
	body := `{"model":"gpt-3.5-turbo-instruct","prompt":"hello world"}`
	req := httptest.NewRequest(http.MethodPost, "/v1/completions",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst CompletionRequest
	if err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if dst.Model != "gpt-3.5-turbo-instruct" {
		t.Errorf("expected model gpt-3.5-turbo-instruct, got %s", dst.Model)
	}
}

func TestDecodeAndValidateJSON_ValidEmbedding(t *testing.T) {
	body := `{"model":"text-embedding-3-small","input":"hello world"}`
	req := httptest.NewRequest(http.MethodPost, "/v1/embeddings",
		strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")

	var dst EmbeddingRequest
	if err := decodeAndValidateJSON(req, &dst, DefaultMaxBodySize); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if dst.Input != "hello world" {
		t.Errorf("expected input 'hello world', got %s", dst.Input)
	}
}

func TestWriteBodyError(t *testing.T) {
	rec := httptest.NewRecorder()
	writeBodyError(rec, &bodyError{
		Status:  http.StatusBadRequest,
		Message: "bad request",
		Type:    "invalid_request_error",
	})
	if rec.Code != http.StatusBadRequest {
		t.Errorf("expected status 400, got %d", rec.Code)
	}
	var resp map[string]interface{}
	if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
}

func TestContentTypeJSON_MissingHeader(t *testing.T) {
	handler := ContentTypeJSON(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))

	// POST without Content-Type header.
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnsupportedMediaType {
		t.Errorf("expected 415, got %d", rec.Code)
	}
}

func TestContentTypeJSON_ValidHeader(t *testing.T) {
	handler := ContentTypeJSON(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))

	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil)
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Errorf("expected 200, got %d", rec.Code)
	}
}

func TestContentTypeJSON_GETPasses(t *testing.T) {
	handler := ContentTypeJSON(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))

	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Errorf("expected 200 for GET, got %d", rec.Code)
	}
}
