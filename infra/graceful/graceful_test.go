package graceful

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/maximhq/bifrost/core/schemas"

	"github.com/kooshapari/bifrost-extensions/infra/circuitbreaker"
)

type mockPlugin struct {
	name               string
	preHookErr         error
	postHookErr        error
	shouldShortCircuit bool
}

func (m *mockPlugin) GetName() string {
	return m.name
}

func (m *mockPlugin) PreRequestHook(_ *schemas.BifrostContext, _ *schemas.BifrostRequest) error {
	return nil
}

func (m *mockPlugin) PreLLMHook(
	_ *schemas.BifrostContext,
	req *schemas.BifrostRequest,
) (*schemas.BifrostRequest, *schemas.LLMPluginShortCircuit, error) {
	if m.shouldShortCircuit {
		return req, &schemas.LLMPluginShortCircuit{}, nil
	}
	return req, nil, m.preHookErr
}

func (m *mockPlugin) PostLLMHook(
	_ *schemas.BifrostContext,
	resp *schemas.BifrostResponse,
	bifrostErr *schemas.BifrostError,
) (*schemas.BifrostResponse, *schemas.BifrostError, error) {
	return resp, bifrostErr, m.postHookErr
}

func (m *mockPlugin) Cleanup() error {
	return nil
}

func testContext() *schemas.BifrostContext {
	return schemas.NewBifrostContext(context.Background(), schemas.NoDeadline)
}

func TestPluginManager_GetPlugins(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2"},
	}

	manager := NewPluginManager(plugins, DefaultConfig(), nil)
	wrapped := manager.GetPlugins()

	if len(wrapped) != len(plugins) {
		t.Errorf("Expected %d plugins, got %d", len(plugins), len(wrapped))
	}
}

func TestPluginManager_ExecutePreHooks_Success(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2"},
	}

	manager := NewPluginManager(plugins, DefaultConfig(), nil)
	req := &schemas.BifrostRequest{}

	resultReq, _, err := manager.ExecutePreLLMHooks(testContext(), req)

	if err != nil {
		t.Errorf("Expected no error, got %v", err)
	}

	if resultReq == nil {
		t.Error("Expected non-nil request")
	}
}

func TestPluginManager_ExecutePreHooks_WithFailure(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2", preHookErr: errors.New("plugin error")},
		&mockPlugin{name: "plugin3"},
	}

	config := DefaultConfig()
	config.FailFast = false
	manager := NewPluginManager(plugins, config, nil)
	req := &schemas.BifrostRequest{}

	resultReq, _, err := manager.ExecutePreLLMHooks(testContext(), req)

	if resultReq == nil {
		t.Error("Expected non-nil request even with plugin failure")
	}

	if err != nil {
		t.Logf("Error logged (expected): %v", err)
	}
}

func TestPluginManager_ExecutePreHooks_FailFast(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2", preHookErr: errors.New("plugin error")},
	}

	config := DefaultConfig()
	config.FailFast = true
	manager := NewPluginManager(plugins, config, nil)
	req := &schemas.BifrostRequest{}

	_, _, err := manager.ExecutePreLLMHooks(testContext(), req)

	if err == nil {
		cb := manager.GetCircuitBreaker("plugin2")
		if cb != nil && cb.State() == circuitbreaker.StateClosed {
			t.Error("Expected error with FailFast enabled, or circuit breaker to be open")
		}
	} else if !strings.Contains(err.Error(), "plugin2") {
		t.Errorf("Expected error to mention plugin2, got: %v", err)
	}
}

func TestPluginManager_ExecutePostHooks_Success(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2"},
	}

	manager := NewPluginManager(plugins, DefaultConfig(), nil)
	resp := &schemas.BifrostResponse{}

	resultResp, _, err := manager.ExecutePostLLMHooks(testContext(), resp, nil)

	if err != nil {
		t.Errorf("Expected no error, got %v", err)
	}

	if resultResp == nil {
		t.Error("Expected non-nil response")
	}
}

func TestPluginManager_ExecutePostHooks_WithFailure(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2", postHookErr: errors.New("plugin error")},
		&mockPlugin{name: "plugin3"},
	}

	config := DefaultConfig()
	config.FailFast = false
	manager := NewPluginManager(plugins, config, nil)
	resp := &schemas.BifrostResponse{}

	resultResp, _, _ := manager.ExecutePostLLMHooks(testContext(), resp, nil)

	if resultResp == nil {
		t.Error("Expected non-nil response even with plugin failure")
	}
}

func TestPluginManager_GetCircuitBreakerStats(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2"},
	}

	manager := NewPluginManager(plugins, DefaultConfig(), nil)
	stats := manager.GetCircuitBreakerStats()

	if len(stats) != len(plugins) {
		t.Errorf("Expected %d circuit breaker stats, got %d", len(plugins), len(stats))
	}

	for _, plugin := range plugins {
		if _, ok := stats[plugin.GetName()]; !ok {
			t.Errorf("Expected stats for plugin %s", plugin.GetName())
		}
	}
}

func TestPluginManager_ResetCircuitBreaker(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
	}

	manager := NewPluginManager(plugins, DefaultConfig(), nil)

	cb := manager.GetCircuitBreaker("plugin1")
	if cb == nil {
		t.Fatal("Expected circuit breaker for plugin1")
	}

	for i := 0; i < 5; i++ {
		cb.RecordResult(false)
	}

	if cb.State() != circuitbreaker.StateOpen {
		t.Error("Expected circuit to be open")
	}

	manager.ResetCircuitBreaker("plugin1")

	if cb.State() != circuitbreaker.StateClosed {
		t.Error("Expected circuit to be closed after reset")
	}
}

func TestPluginManager_ResetAllCircuitBreakers(t *testing.T) {
	plugins := []schemas.LLMPlugin{
		&mockPlugin{name: "plugin1"},
		&mockPlugin{name: "plugin2"},
	}

	manager := NewPluginManager(plugins, DefaultConfig(), nil)

	for _, plugin := range plugins {
		cb := manager.GetCircuitBreaker(plugin.GetName())
		for i := 0; i < 5; i++ {
			cb.RecordResult(false)
		}
	}

	manager.ResetAllCircuitBreakers()

	for _, plugin := range plugins {
		cb := manager.GetCircuitBreaker(plugin.GetName())
		if cb.State() != circuitbreaker.StateClosed {
			t.Errorf("Expected circuit for %s to be closed", plugin.GetName())
		}
	}
}
