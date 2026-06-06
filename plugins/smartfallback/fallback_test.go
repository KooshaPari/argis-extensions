package smartfallback

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
	if p.GetName() != "smart-fallback" {
		t.Errorf("expected name 'smart-fallback', got %s", p.GetName())
	}
}

func TestPluginPreHook(t *testing.T) {
	cfg := DefaultConfig()
	p := New(cfg)

	req := &schemas.BifrostRequest{
		ChatRequest: &schemas.ChatRequest{
			Model: "gpt-4",
			Messages: []schemas.Message{
				{
					Role:    "user",
					Content: "write a function to sort an array",
				},
			},
		},
	}

	result, shortCircuit, err := p.PreHook(context.Background(), req)
	if err != nil {
		t.Fatalf("PreHook returned error: %v", err)
	}
	if shortCircuit != nil {
		t.Error("PreHook should not short-circuit")
	}
	if result == nil {
		t.Error("PreHook should return a request")
	}
}

func TestPluginPostHook(t *testing.T) {
	cfg := DefaultConfig()
	p := New(cfg)

	resp := &schemas.BifrostResponse{
		ChatResponse: &schemas.ChatResponse{
			ID: "test-id",
		},
	}

	result, bifrostErr, err := p.PostHook(context.Background(), resp, nil)
	if err != nil {
		t.Fatalf("PostHook returned error: %v", err)
	}
	if bifrostErr != nil {
		t.Error("PostHook should not return BifrostError on success")
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

func TestExponentialBackoff(t *testing.T) {
	cfg := DefaultConfig()
	b := NewExponentialBackoffStrategy("gpt-4", cfg)

	if b == nil {
		t.Fatal("NewExponentialBackoffStrategy returned nil")
	}
}

func TestBudgetStrategy(t *testing.T) {
	cfg := DefaultConfig()
	budget := 100.0
	s := NewBudgetAwareStrategy(cfg, budget)

	if s == nil {
		t.Fatal("NewBudgetAwareStrategy returned nil")
	}
}

func TestTaskRuleEngine(t *testing.T) {
	engine := NewTaskRuleEngine()

	if engine == nil {
		t.Fatal("NewTaskRuleEngine returned nil")
	}

	// Test getting preferred models for a task
	models := engine.GetPreferredModels("codegen")
	if len(models) == 0 {
		t.Error("expected at least one preferred model")
	}

	// Test checking if model is available
	available := engine.IsModelAvailable("codegen", "gpt-4")
	if !available {
		t.Error("expected gpt-4 to be available for codegen")
	}
}

