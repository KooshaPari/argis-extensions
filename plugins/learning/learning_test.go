package learning

import (
	"context"
	"testing"

	"github.com/maximhq/bifrost/core/schemas"
)

func TestNewPlugin(t *testing.T) {
	cfg := DefaultConfig()
	p := New(cfg)

	if p == nil {
		t.Fatal("New returned nil")
	}
	if p.GetName() != "learning-system" {
		t.Errorf("expected name 'learning-system', got %s", p.GetName())
	}
}

func TestPluginPreHook(t *testing.T) {
	cfg := DefaultConfig()
	p := New(cfg)

	ctx := context.Background()
	req := &schemas.BifrostRequest{
		ChatRequest: &schemas.ChatRequest{
			Model: "gpt-4",
			Messages: []schemas.Message{
				{
					Role:    "user",
					Content: "test message",
				},
			},
		},
	}

	result, shortCircuit, err := p.PreHook(ctx, req)
	if err != nil {
		t.Fatalf("PreHook returned error: %v", err)
	}
	if shortCircuit != nil {
		t.Error("PreHook should not short-circuit")
	}
	if result != req {
		t.Error("PreHook should return the same request")
	}
}

func TestPluginPostHook(t *testing.T) {
	cfg := DefaultConfig()
	p := New(cfg)

	ctx := context.Background()
	resp := &schemas.BifrostResponse{
		ChatResponse: &schemas.ChatResponse{
			ID: "test-id",
			Choices: []schemas.ChatResponseChoice{
				{
					Index: 0,
					Message: schemas.ChatMessage{
						Role:    "assistant",
						Content: "response content",
					},
				},
			},
		},
	}

	result, bifrostErr, err := p.PostHook(ctx, resp, nil)
	if err != nil {
		t.Fatalf("PostHook returned error: %v", err)
	}
	if bifrostErr != nil {
		t.Error("PostHook should not return BifrostError")
	}
	if result != resp {
		t.Error("PostHook should return the same response")
	}
}

func TestPluginCleanup(t *testing.T) {
	cfg := DefaultConfig()
	p := New(cfg)

	err := p.Cleanup()
	if err != nil {
		t.Errorf("Cleanup returned error: %v", err)
	}
}

func TestTransportInterceptor(t *testing.T) {
	cfg := DefaultConfig()
	p := New(cfg)

	ctx := context.Background()
	req := &schemas.BifrostRequest{
		ChatRequest: &schemas.ChatRequest{
			Model: "gpt-4",
			Messages: []schemas.Message{
				{
					Role:    "user",
					Content: "test",
				},
			},
		},
	}

	resultReq, shortCircuit, err := p.TransportInterceptor(ctx, req)
	if err != nil {
		t.Fatalf("TransportInterceptor returned error: %v", err)
	}
	if shortCircuit != nil {
		t.Error("TransportInterceptor should not short-circuit")
	}
	if resultReq == nil {
		t.Error("TransportInterceptor should return request")
	}
}

