package account

import (
	"context"
	"testing"
	"time"

	"github.com/maximhq/bifrost/core/schemas"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/mock"
)

// MockAccount is a mock implementation of schemas.Account
type MockAccount struct {
	mock.Mock
}

func (m *MockAccount) GetConfiguredProviders() ([]schemas.Provider, error) {
	args := m.Called()
	return args.Get(0).([]schemas.Provider), args.Error(1)
}

func (m *MockAccount) GetConfigForProvider(provider schemas.Provider) (*schemas.ProviderConfig, error) {
	args := m.Called(provider)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*schemas.ProviderConfig), args.Error(1)
}

func (m *MockAccount) GetKeysForProvider(ctx *context.Context, provider schemas.Provider) ([]schemas.Key, error) {
	args := m.Called(ctx, provider)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).([]schemas.Key), args.Error(1)
}

func TestNewEnhancedAccount(t *testing.T) {
	account := NewEnhancedAccount(nil)

	assert.NotNil(t, account)
	assert.NotNil(t, account.configs)
	assert.NotNil(t, account.keys)
	assert.Nil(t, account.fallback)
}

func TestNewEnhancedAccount_NilFallback(t *testing.T) {
	account := NewEnhancedAccount(nil)

	assert.NotNil(t, account)
	assert.NotNil(t, account.configs)
	assert.NotNil(t, account.keys)
	assert.Nil(t, account.fallback)
}

func TestGetConfiguredProviders_NoFallback(t *testing.T) {
	account := NewEnhancedAccount(nil)

	providers, err := account.GetConfiguredProviders()

	assert.NoError(t, err)
	assert.Empty(t, providers)
}

func TestGetConfiguredProviders_WithConfigs(t *testing.T) {
	account := NewEnhancedAccount(nil)
	account.SetConfig(schemas.ProviderOpenAI, &schemas.ProviderConfig{})
	account.SetConfig(schemas.ProviderAnthropic, &schemas.ProviderConfig{})

	providers, err := account.GetConfiguredProviders()

	assert.NoError(t, err)
	assert.Len(t, providers, 2)
	assert.Contains(t, providers, schemas.ProviderOpenAI)
	assert.Contains(t, providers, schemas.ProviderAnthropic)
}

func TestGetConfiguredProviders_WithFallback(t *testing.T) {
	account := NewEnhancedAccount(nil)
	account.SetConfig(schemas.Gemini, &schemas.ProviderConfig{})

	providers, err := account.GetConfiguredProviders()

	assert.NoError(t, err)
	assert.Len(t, providers, 1)
	assert.Contains(t, providers, schemas.Gemini)
}

func TestGetConfiguredProviders_FallbackError(t *testing.T) {
	account := NewEnhancedAccount(nil)
	account.SetConfig(schemas.Gemini, &schemas.ProviderConfig{})

	providers, err := account.GetConfiguredProviders()

	assert.NoError(t, err)
	assert.Len(t, providers, 1)
	assert.Contains(t, providers, schemas.Gemini)
}

func TestGetConfigForProvider_NoFallback(t *testing.T) {
	account := NewEnhancedAccount(nil)

	config, err := account.GetConfigForProvider(schemas.ProviderOpenAI)

	assert.NoError(t, err)
	assert.NotNil(t, config)
	// Should return default config
	assert.Equal(t, 60, config.NetworkConfig.DefaultRequestTimeoutInSeconds)
	assert.Equal(t, 3, config.NetworkConfig.MaxRetries)
}

func TestGetConfigForProvider_WithConfig(t *testing.T) {
	account := NewEnhancedAccount(nil)
	customConfig := &schemas.ProviderConfig{
		NetworkConfig: schemas.NetworkConfig{
			DefaultRequestTimeoutInSeconds: 120,
			MaxRetries:                     5,
		},
	}
	account.SetConfig(schemas.ProviderOpenAI, customConfig)

	config, err := account.GetConfigForProvider(schemas.ProviderOpenAI)

	assert.NoError(t, err)
	assert.NotNil(t, config)
	assert.Equal(t, 120, config.NetworkConfig.DefaultRequestTimeoutInSeconds)
	assert.Equal(t, 5, config.NetworkConfig.MaxRetries)
}

func TestGetConfigForProvider_WithFallback(t *testing.T) {
	account := NewEnhancedAccount(nil)
	customConfig := &schemas.ProviderConfig{
		NetworkConfig: schemas.NetworkConfig{
			DefaultRequestTimeoutInSeconds: 90,
		},
	}
	account.SetConfig(schemas.ProviderOpenAI, customConfig)

	config, err := account.GetConfigForProvider(schemas.ProviderOpenAI)

	assert.NoError(t, err)
	assert.NotNil(t, config)
	assert.Equal(t, 90, config.NetworkConfig.DefaultRequestTimeoutInSeconds)
}

func TestGetKeysForProvider_NoFallback(t *testing.T) {
	account := NewEnhancedAccount(nil)
	ctx := context.Background()

	keys, err := account.GetKeysForProvider(ctx, schemas.ProviderOpenAI)

	assert.NoError(t, err)
	assert.Nil(t, keys)
}

func TestGetKeysForProvider_WithKeys(t *testing.T) {
	account := NewEnhancedAccount(nil)
	ctx := context.Background()
	testKeys := []schemas.Key{
		{ID: "key1", Value: "secret1"},
		{ID: "key2", Value: "secret2"},
	}
	account.SetKeys(schemas.ProviderOpenAI, testKeys)

	keys, err := account.GetKeysForProvider(ctx, schemas.ProviderOpenAI)

	assert.NoError(t, err)
	assert.Len(t, keys, 2)
	assert.Equal(t, "key1", keys[0].ID)
	assert.Equal(t, "key2", keys[1].ID)
}

func TestGetKeysForProvider_WithFallback(t *testing.T) {
	account := NewEnhancedAccount(nil)
	ctx := context.Background()
	testKeys := []schemas.Key{
		{ID: "test-key", Value: "test-secret"},
	}
	account.SetKeys(schemas.ProviderOpenAI, testKeys)

	keys, err := account.GetKeysForProvider(ctx, schemas.ProviderOpenAI)

	assert.NoError(t, err)
	assert.Len(t, keys, 1)
	assert.Equal(t, "test-key", keys[0].ID)
}

func TestSetConfig(t *testing.T) {
	account := NewEnhancedAccount(nil)
	config := &schemas.ProviderConfig{
		NetworkConfig: schemas.NetworkConfig{
			DefaultRequestTimeoutInSeconds: 100,
		},
	}

	account.SetConfig(schemas.ProviderOpenAI, config)

	retrieved, err := account.GetConfigForProvider(schemas.ProviderOpenAI)
	assert.NoError(t, err)
	assert.Equal(t, 100, retrieved.NetworkConfig.DefaultRequestTimeoutInSeconds)
}

func TestSetKeys(t *testing.T) {
	account := NewEnhancedAccount(nil)
	keys := []schemas.Key{
		{ID: "key1", Value: "value1"},
	}

	account.SetKeys(schemas.ProviderOpenAI, keys)

	ctx := context.Background()
	retrieved, err := account.GetKeysForProvider(ctx, schemas.ProviderOpenAI)
	assert.NoError(t, err)
	assert.Len(t, retrieved, 1)
	assert.Equal(t, "key1", retrieved[0].ID)
}

func TestDefaultProviderConfig(t *testing.T) {
	config := defaultProviderConfig()

	assert.NotNil(t, config)
	assert.Equal(t, 60, config.NetworkConfig.DefaultRequestTimeoutInSeconds)
	assert.Equal(t, 3, config.NetworkConfig.MaxRetries)
	assert.Equal(t, 500*time.Millisecond, config.NetworkConfig.RetryBackoffInitial)
	assert.Equal(t, 5*time.Second, config.NetworkConfig.RetryBackoffMax)
	assert.Equal(t, 10, config.ConcurrencyAndBuffer.Concurrency)
	assert.Equal(t, 100, config.ConcurrencyAndBuffer.BufferSize)
}

func TestConcurrentAccess(t *testing.T) {
	account := NewEnhancedAccount(nil)

	// Test concurrent writes
	done := make(chan bool, 10)
	for i := 0; i < 10; i++ {
		go func(idx int) {
			config := &schemas.ProviderConfig{
				NetworkConfig: schemas.NetworkConfig{
					DefaultRequestTimeoutInSeconds: idx,
				},
			}
			account.SetConfig(schemas.ProviderOpenAI, config)
			account.GetConfigForProvider(schemas.ProviderOpenAI)
			done <- true
		}(i)
	}

	// Wait for all goroutines
	for i := 0; i < 10; i++ {
		<-done
	}

	// Should not panic
	config, err := account.GetConfigForProvider(schemas.ProviderOpenAI)
	assert.NoError(t, err)
	assert.NotNil(t, config)
}
