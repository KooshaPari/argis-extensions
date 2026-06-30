package api

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"

	"github.com/go-playground/validator/v10"
)

// DefaultMaxBodySize is the default maximum request body size in bytes (10 MB).
const DefaultMaxBodySize int64 = 10 * 1024 * 1024

// validate is the singleton validator instance.
var validate = validator.New()

// decodeAndValidateJSON reads the request body (limited to maxBytes), decodes
// it into the target struct, and runs struct-tag validation via
// go-playground/validator. It returns a structured HTTP error on failure.
func decodeAndValidateJSON(r *http.Request, target interface{}, maxBytes int64) error {
	if maxBytes <= 0 {
		maxBytes = DefaultMaxBodySize
	}

	// Limit request body size (defense-in-depth against oversized payloads).
	r.Body = http.MaxBytesReader(nil, r.Body, maxBytes)

	// Decode JSON.
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(target); err != nil {
		return &bodyError{
			Status:  http.StatusBadRequest,
			Message: fmt.Sprintf("invalid request body: %v", err),
			Type:    "invalid_request_error",
		}
	}

	// Verify there is no trailing data.
	if err := dec.Decode(&struct{}{}); err != io.EOF {
		return &bodyError{
			Status:  http.StatusBadRequest,
			Message: "request body contains trailing data",
			Type:    "invalid_request_error",
		}
	}

	// Run struct-tag validation.
	if err := validate.Struct(target); err != nil {
		if verrs, ok := err.(validator.ValidationErrors); ok {
			var details []string
			for _, ve := range verrs {
				details = append(details, fmt.Sprintf("%s: %s", ve.Namespace(), ve.Tag()))
			}
			return &bodyError{
				Status:  http.StatusUnprocessableEntity,
				Message: fmt.Sprintf("validation failed: %v", details),
				Type:    "validation_error",
			}
		}
		return &bodyError{
			Status:  http.StatusUnprocessableEntity,
			Message: fmt.Sprintf("validation failed: %v", err),
			Type:    "validation_error",
		}
	}

	return nil
}

// bodyError is a structured error for request body failures.
type bodyError struct {
	Status  int
	Message string
	Type    string
}

func (e *bodyError) Error() string { return e.Message }

// bodyErrorResponse writes a JSON error response for body errors.
func writeBodyError(w http.ResponseWriter, err error) {
	if be, ok := err.(*bodyError); ok {
		writeError(w, be.Status, be.Message, be.Type)
		return
	}
	writeError(w, http.StatusBadRequest, err.Error(), "invalid_request_error")
}

// ContentTypeJSON ensures the request has Content-Type: application/json
// for endpoints that require JSON bodies.
func ContentTypeJSON(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodPost || r.Method == http.MethodPut || r.Method == http.MethodPatch {
			if ct := r.Header.Get("Content-Type"); ct != "application/json" {
				writeError(w, http.StatusUnsupportedMediaType,
					"Content-Type must be application/json", "invalid_request_error")
				return
			}
		}
		next.ServeHTTP(w, r)
	})
}

// ChatCompletionRequest is the expected shape of a /v1/chat/completions body.
type ChatCompletionRequest struct {
	Model       string    `json:"model" validate:"required"`
	Messages    []Message `json:"messages" validate:"required,min=1,dive"`
	MaxTokens   int       `json:"max_tokens,omitempty" validate:"omitempty,min=1"`
	Temperature float64   `json:"temperature,omitempty" validate:"omitempty,min=0,max=2"`
	Stream      bool      `json:"stream,omitempty"`
}

// Message is a single chat message.
type Message struct {
	Role    string `json:"role" validate:"required,oneof=system user assistant tool"`
	Content string `json:"content" validate:"required"`
}

// CompletionRequest is the expected shape of a /v1/completions body.
type CompletionRequest struct {
	Model       string  `json:"model" validate:"required"`
	Prompt      string  `json:"prompt" validate:"required"`
	MaxTokens   int     `json:"max_tokens,omitempty" validate:"omitempty,min=1"`
	Temperature float64 `json:"temperature,omitempty" validate:"omitempty,min=0,max=2"`
}

// EmbeddingRequest is the expected shape of a /v1/embeddings body.
type EmbeddingRequest struct {
	Model string `json:"model" validate:"required"`
	Input string `json:"input" validate:"required"`
}
