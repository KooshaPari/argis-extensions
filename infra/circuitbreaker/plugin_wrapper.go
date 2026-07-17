// Package circuitbreaker provides plugin wrapper with circuit breaker protection
package circuitbreaker

import (
	"context"
	"errors"
	"fmt"
	"sync"

	"github.com/maximhq/bifrost/core/schemas"
)

// PluginWrapper wraps an LLM plugin with circuit breaker protection.
type PluginWrapper struct {
	plugin         schemas.LLMPlugin
	circuitBreaker *CircuitBreaker
	mu             sync.RWMutex
}

// NewPluginWrapper creates a new plugin wrapper with circuit breaker.
func NewPluginWrapper(plugin schemas.LLMPlugin, config *Config) *PluginWrapper {
	name := fmt.Sprintf("plugin-%s", plugin.GetName())
	cb := New(name, config)
	return &PluginWrapper{
		plugin:         plugin,
		circuitBreaker: cb,
	}
}

// GetName returns the plugin name.
func (pw *PluginWrapper) GetName() string {
	return pw.plugin.GetName()
}

// PreRequestHook wraps the plugin PreRequestHook with circuit breaker protection.
func (pw *PluginWrapper) PreRequestHook(
	ctx *schemas.BifrostContext,
	req *schemas.BifrostRequest,
) error {
	var hookErr error
	parent := parentContext(ctx)
	cbErr := pw.circuitBreaker.Execute(parent, func() error {
		hookErr = pw.plugin.PreRequestHook(ctx, req)
		return hookErr
	})
	if cbErr != nil && isCircuitOpen(cbErr, pw.circuitBreaker.name) {
		return nil
	}
	if cbErr != nil {
		return cbErr
	}
	return hookErr
}

// PreLLMHook wraps the plugin PreLLMHook with circuit breaker protection.
func (pw *PluginWrapper) PreLLMHook(
	ctx *schemas.BifrostContext,
	req *schemas.BifrostRequest,
) (*schemas.BifrostRequest, *schemas.LLMPluginShortCircuit, error) {
	var resultReq *schemas.BifrostRequest
	var shortCircuit *schemas.LLMPluginShortCircuit
	var hookErr error

	parent := parentContext(ctx)
	cbErr := pw.circuitBreaker.Execute(parent, func() error {
		resultReq, shortCircuit, hookErr = pw.plugin.PreLLMHook(ctx, req)
		return hookErr
	})

	if cbErr != nil && isCircuitOpen(cbErr, pw.circuitBreaker.name) {
		return req, nil, nil
	}
	if cbErr != nil {
		return resultReq, shortCircuit, cbErr
	}
	return resultReq, shortCircuit, hookErr
}

// PostLLMHook wraps the plugin PostLLMHook with circuit breaker protection.
func (pw *PluginWrapper) PostLLMHook(
	ctx *schemas.BifrostContext,
	resp *schemas.BifrostResponse,
	bifrostErr *schemas.BifrostError,
) (*schemas.BifrostResponse, *schemas.BifrostError, error) {
	var resultResp *schemas.BifrostResponse
	var resultErr *schemas.BifrostError
	var hookErr error

	parent := parentContext(ctx)
	cbErr := pw.circuitBreaker.Execute(parent, func() error {
		resultResp, resultErr, hookErr = pw.plugin.PostLLMHook(ctx, resp, bifrostErr)
		if hookErr != nil {
			return hookErr
		}
		if resultErr != nil && resultErr.StatusCode != nil {
			statusCode := *resultErr.StatusCode
			if statusCode >= 500 && statusCode < 600 {
				return errors.New(resultErr.GetErrorString())
			}
		}
		return nil
	})

	if cbErr != nil && isCircuitOpen(cbErr, pw.circuitBreaker.name) {
		return resp, bifrostErr, nil
	}
	if cbErr != nil {
		return resultResp, resultErr, cbErr
	}
	return resultResp, resultErr, hookErr
}

// Cleanup wraps plugin Cleanup.
func (pw *PluginWrapper) Cleanup() error {
	return pw.plugin.Cleanup()
}

// GetCircuitBreaker returns the underlying circuit breaker.
func (pw *PluginWrapper) GetCircuitBreaker() *CircuitBreaker {
	return pw.circuitBreaker
}

// WrapPlugins wraps multiple LLM plugins with circuit breaker protection.
func WrapPlugins(plugins []schemas.LLMPlugin, config *Config) []schemas.LLMPlugin {
	wrapped := make([]schemas.LLMPlugin, 0, len(plugins))
	for _, plugin := range plugins {
		wrapped = append(wrapped, NewPluginWrapper(plugin, config))
	}
	return wrapped
}

func parentContext(ctx *schemas.BifrostContext) context.Context {
	if ctx == nil {
		return context.Background()
	}
	return ctx.GetParentCtxWithUserValues()
}

func isCircuitOpen(err error, name string) bool {
	return err != nil && err.Error() == fmt.Sprintf("circuit breaker %s is open", name)
}
