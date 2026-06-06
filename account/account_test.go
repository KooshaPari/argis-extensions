package account

import (
	"context"
	"testing"

	"github.com/maximhq/bifrost/core/schemas"
	"github.com/stretchr/testify/assert"
)

func TestNewEnhancedAccount(t *testing.T) {
	baseAccount := &schemas.Account{
		ID:        "test-account",
		Providers: []schemas.Provider{},
	}
	account := schemas.NewEnhancedAccount(baseAccount)

	assert.NotNil(t, account)
	assert.Equal(t, baseAccount, account.Account)
	assert.Nil(t, account.GetFallback())
}

func TestNewEnhancedAccount_NilBase(t *testing.T) {
	account := schemas.NewEnhancedAccount(nil)

	assert.NotNil(t, account)
	assert.Nil(t, account.Account)
	assert.Nil(t, account.GetFallback())
}

func TestGetConfiguredProviders_NoConfigs(t *testing.T) {
	baseAccount := &schemas.Account{
		ID:      "test",
		Configs: []schemas.ProviderConfig{},
	}
	account := schemas.NewEnhancedAccount(baseAccount)

	providers := account.GetConfiguredProviders()

	assert.Empty(t, providers)
}

func TestGetConfiguredProviders_WithConfigs(t *testing.T) {
	baseAccount := &schemas.Account{
		ID: "test",
		Configs: []schemas.ProviderConfig{
			{Provider: schemas.ProviderOpenAI},
			{Provider: schemas.ProviderAnthropic},
		},
	}
	account := schemas.NewEnhancedAccount(baseAccount)

	providers := account.GetConfiguredProviders()

	assert.Len(t, providers, 2)
	assert.Contains(t, providers, schemas.ProviderOpenAI)
	assert.Contains(t, providers, schemas.ProviderAnthropic)
}

func TestGetConfiguredProviders_WithFallback(t *testing.T) {
	baseAccount := &schemas.Account{
		ID: "test",
		Configs: []schemas.ProviderConfig{
			{Provider: schemas.ProviderGemini},
		},
	}
	fallbackAccount := &schemas.Account{
		ID: "fallback",
		Configs: []schemas.ProviderConfig{
			{Provider: schemas.ProviderOpenAI},
			{Provider: schemas.ProviderAnthropic},
		},
	}
	account := schemas.NewEnhancedAccount(baseAccount)
	account.SetFallback(fallbackAccount)

	providers := account.GetConfiguredProviders()

	assert.Len(t, providers, 1)
	assert.Contains(t, providers, schemas.ProviderGemini)
}

func TestGetKeysForProvider(t *testing.T) {
	baseAccount := &schemas.Account{
		ID: "test",
		Keys: []schemas.Key{
			{ID: "key1", Provider: schemas.ProviderOpenAI, IsActive: true},
			{ID: "key2", Provider: schemas.ProviderAnthropic, IsActive: true},
			{ID: "key3", Provider: schemas.ProviderOpenAI, IsActive: false},
		},
	}
	account := schemas.NewEnhancedAccount(baseAccount)

	ctx := context.Background()
	keys, err := account.GetKeysForProvider(ctx, schemas.ProviderOpenAI)

	assert.NoError(t, err)
	assert.Len(t, keys, 1)
	assert.Equal(t, "key1", keys[0].ID)
}

func TestGetConfigForProvider(t *testing.T) {
	baseAccount := &schemas.Account{
		ID: "test",
		Configs: []schemas.ProviderConfig{
			{
				Provider: schemas.ProviderOpenAI,
				BaseURL:  "https://api.openai.com",
			},
		},
	}
	account := schemas.NewEnhancedAccount(baseAccount)

	config, found := account.GetConfigForProvider(schemas.ProviderOpenAI)

	assert.True(t, found)
	assert.NotNil(t, config)
	assert.Equal(t, schemas.ProviderOpenAI, config.Provider)
}

func TestSetFallback(t *testing.T) {
	baseAccount := &schemas.Account{ID: "base"}
	fallbackAccount := &schemas.Account{ID: "fallback"}

	account := schemas.NewEnhancedAccount(baseAccount)
	account.SetFallback(fallbackAccount)

	retrieved := account.GetFallback()
	assert.Equal(t, fallbackAccount, retrieved)
}
