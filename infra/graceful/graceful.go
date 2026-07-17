// Package graceful provides graceful degradation for plugin failures
package graceful

import (
	"fmt"
	"sync"

	"github.com/maximhq/bifrost/core/schemas"

	"github.com/kooshapari/bifrost-extensions/infra/circuitbreaker"
)

// PluginManager manages plugins with graceful degradation
type PluginManager struct {
	plugins         []schemas.LLMPlugin
	wrappedPlugins  []schemas.LLMPlugin
	circuitBreakers map[string]*circuitbreaker.CircuitBreaker
	config          *Config
	mu              sync.RWMutex
	logger          schemas.Logger
}

// Config configures the plugin manager
type Config struct {
	// CircuitBreakerConfig configures circuit breakers for plugins
	CircuitBreakerConfig *circuitbreaker.Config
	// FailFast if true, returns error immediately on plugin failure
	// If false, continues with degraded functionality
	FailFast bool
}

// DefaultConfig returns sensible defaults
func DefaultConfig() *Config {
	return &Config{
		CircuitBreakerConfig: circuitbreaker.DefaultConfig(),
		FailFast:             false, // Graceful degradation by default
	}
}

// NewPluginManager creates a new plugin manager with graceful degradation
func NewPluginManager(plugins []schemas.LLMPlugin, config *Config, logger schemas.Logger) *PluginManager {
	if config == nil {
		config = DefaultConfig()
	}

	wrapped := circuitbreaker.WrapPlugins(plugins, config.CircuitBreakerConfig)

	breakers := make(map[string]*circuitbreaker.CircuitBreaker)
	for i, plugin := range plugins {
		wrapper, ok := wrapped[i].(*circuitbreaker.PluginWrapper)
		if ok {
			breakers[plugin.GetName()] = wrapper.GetCircuitBreaker()
		}
	}

	return &PluginManager{
		plugins:         plugins,
		wrappedPlugins:  wrapped,
		circuitBreakers: breakers,
		config:          config,
		logger:          logger,
	}
}

// GetPlugins returns the wrapped plugins with circuit breaker protection
func (pm *PluginManager) GetPlugins() []schemas.LLMPlugin {
	pm.mu.RLock()
	defer pm.mu.RUnlock()
	return pm.wrappedPlugins
}

// ExecutePreLLMHooks executes all plugin PreLLMHooks with graceful degradation
func (pm *PluginManager) ExecutePreLLMHooks(
	ctx *schemas.BifrostContext,
	req *schemas.BifrostRequest,
) (*schemas.BifrostRequest, *schemas.LLMPluginShortCircuit, error) {
	pm.mu.RLock()
	plugins := pm.wrappedPlugins
	pm.mu.RUnlock()

	lastReq := req
	var lastShortCircuit *schemas.LLMPluginShortCircuit
	var lastErr error

	for _, plugin := range plugins {
		if err := plugin.PreRequestHook(ctx, lastReq); err != nil {
			if pm.logger != nil {
				pm.logger.Warn("Plugin PreRequestHook failed",
					"plugin", plugin.GetName(),
					"error", err.Error(),
				)
			}
			if pm.config != nil && pm.config.FailFast {
				return nil, nil, fmt.Errorf("plugin %s PreRequestHook failed: %w", plugin.GetName(), err)
			}
			lastErr = err
		}

		pluginReq, shortCircuit, err := plugin.PreLLMHook(ctx, lastReq)
		if err != nil {
			if pm.logger != nil {
				pm.logger.Warn("Plugin PreLLMHook failed",
					"plugin", plugin.GetName(),
					"error", err.Error(),
				)
			}
			if pm.config != nil && pm.config.FailFast {
				return nil, nil, fmt.Errorf("plugin %s PreLLMHook failed: %w", plugin.GetName(), err)
			}
			lastErr = err
			continue
		}

		if shortCircuit != nil {
			lastShortCircuit = shortCircuit
		}
		if pluginReq != nil {
			lastReq = pluginReq
		}
	}

	return lastReq, lastShortCircuit, lastErr
}

// ExecutePostLLMHooks executes all plugin PostLLMHooks with graceful degradation
func (pm *PluginManager) ExecutePostLLMHooks(
	ctx *schemas.BifrostContext,
	resp *schemas.BifrostResponse,
	bifrostErr *schemas.BifrostError,
) (*schemas.BifrostResponse, *schemas.BifrostError, error) {
	pm.mu.RLock()
	plugins := pm.wrappedPlugins
	pm.mu.RUnlock()

	lastResp := resp
	var lastErr *schemas.BifrostError
	var lastHookErr error

	for _, plugin := range plugins {
		pluginResp, pluginErr, hookErr := plugin.PostLLMHook(ctx, lastResp, bifrostErr)
		if hookErr != nil {
			if pm.logger != nil {
				pm.logger.Warn("Plugin PostLLMHook failed",
					"plugin", plugin.GetName(),
					"error", hookErr.Error(),
				)
			}
			if pm.config != nil && pm.config.FailFast {
				return nil, nil, fmt.Errorf("plugin %s PostLLMHook failed: %w", plugin.GetName(), hookErr)
			}
			lastHookErr = hookErr
		}
		if pluginResp != nil {
			lastResp = pluginResp
		}
		if pluginErr != nil {
			lastErr = pluginErr
		}
	}

	return lastResp, lastErr, lastHookErr
}

// GetCircuitBreakerStats returns statistics for all circuit breakers
func (pm *PluginManager) GetCircuitBreakerStats() map[string]circuitbreaker.Stats {
	pm.mu.RLock()
	defer pm.mu.RUnlock()

	stats := make(map[string]circuitbreaker.Stats)
	for name, cb := range pm.circuitBreakers {
		stats[name] = cb.GetStats()
	}
	return stats
}

// GetCircuitBreaker returns circuit breaker for a specific plugin
func (pm *PluginManager) GetCircuitBreaker(pluginName string) *circuitbreaker.CircuitBreaker {
	pm.mu.RLock()
	defer pm.mu.RUnlock()
	return pm.circuitBreakers[pluginName]
}

// ResetCircuitBreaker resets circuit breaker for a specific plugin
func (pm *PluginManager) ResetCircuitBreaker(pluginName string) {
	pm.mu.RLock()
	cb := pm.circuitBreakers[pluginName]
	pm.mu.RUnlock()

	if cb != nil {
		cb.Reset()
		if pm.logger != nil {
			pm.logger.Info("Circuit breaker reset", "plugin", pluginName)
		}
	}
}

// ResetAllCircuitBreakers resets all circuit breakers
func (pm *PluginManager) ResetAllCircuitBreakers() {
	pm.mu.RLock()
	defer pm.mu.RUnlock()

	for name, cb := range pm.circuitBreakers {
		cb.Reset()
		if pm.logger != nil {
			pm.logger.Info("Circuit breaker reset", "plugin", name)
		}
	}
}
