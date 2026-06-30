package config

import (
	"testing"
	"time"
)

// TestConfigVersionTracking tests configuration version tracking.
func TestConfigVersionTracking(t *testing.T) {
	cfg := NewVersionedConfig(DefaultConfig())

	// Version assigned on load
	if cfg.Version() == "" {
		t.Error("expected version to be non-empty")
	}
	if cfg.Hash() == "" {
		t.Error("expected hash to be non-empty")
	}

	initialVersion := cfg.Version()
	initialHash := cfg.Hash()

	// Modify the config directly, then record the change.
	cfg.Server.Port = 9090
	cfg.RecordChange("changed port")

	newVersion := cfg.Version()
	newHash := cfg.Hash()

	if newVersion == initialVersion {
		t.Error("expected version to change after modifying config + RecordChange")
	}
	if newHash == initialHash {
		t.Error("expected hash to change after modifying config + RecordChange")
	}
}

// TestConfigChangeHistory tests configuration change history.
func TestConfigChangeHistory(t *testing.T) {
	cfg := NewVersionedConfig(DefaultConfig())

	// Should start with empty history
	if len(cfg.ChangeHistory()) != 0 {
		t.Errorf("expected empty history, got %d entries", len(cfg.ChangeHistory()))
	}

	// Make several changes
	cfg.RecordChange("changed port to 9090")
	cfg.RecordChange("changed port to 8080")
	cfg.RecordChange("changed port to 7070")

	// Verify changes logged
	history := cfg.ChangeHistory()
	if len(history) < 3 {
		t.Errorf("expected at least 3 history entries, got %d", len(history))
	}

	// Verify history is queryable by time
	since := cfg.ChangeHistorySince(time.Now().Add(-1 * time.Hour))
	if len(since) != 3 {
		t.Errorf("expected 3 recent changes, got %d", len(since))
	}

	// Verify no changes from the future
	future := cfg.ChangeHistorySince(time.Now().Add(1 * time.Hour))
	if len(future) != 0 {
		t.Errorf("expected 0 future changes, got %d", len(future))
	}
}

// TestConfigVersionComparison tests configuration version comparison.
func TestConfigVersionComparison(t *testing.T) {
	baseCfg := NewVersionedConfig(DefaultConfig())

	// Create a modified config
	modifiedDefault := DefaultConfig()
	modifiedDefault.Server.Port = 9090
	modifiedDefault.Routing.RouteLLM.Enabled = true
	modifiedCfg := NewVersionedConfig(modifiedDefault)

	// Compare configurations
	comparison := baseCfg.Compare(modifiedCfg)
	if comparison == nil {
		t.Fatal("expected comparison to be non-nil")
	}

	// Verify modified fields are detected
	modified := comparison.ModifiedFields()
	if len(modified) == 0 {
		t.Error("expected modified fields to be detected")
	}

	// Verify specific fields
	foundPort := false
	foundRouter := false
	for _, field := range modified {
		if field == "server.port" {
			foundPort = true
		}
		if field == "routing.routellm.enabled" {
			foundRouter = true
		}
	}
	if !foundPort {
		t.Error("expected server.port to be in modified fields")
	}
	if !foundRouter {
		t.Error("expected routing.routellm.enabled to be in modified fields")
	}

	// Verify no breaking changes
	breaking := comparison.BreakingChanges()
	if breaking == nil {
		t.Error("expected BreakingChanges() to return non-nil slice")
	}

	// Verify no additive changes
	additive := comparison.AdditiveChanges()
	if additive == nil {
		t.Error("expected AdditiveChanges() to return non-nil slice")
	}

	// Verify migration path generated
	migrationPath := comparison.MigrationPath()
	if migrationPath == "" {
		t.Error("expected migration path to be non-empty")
	}

	// Compare identical configs
	identicalCfg := NewVersionedConfig(DefaultConfig())
	sameComparison := baseCfg.Compare(identicalCfg)
	modifiedIdentical := sameComparison.ModifiedFields()
	if len(modifiedIdentical) != 0 {
		t.Errorf("expected no modified fields for identical configs, got %v", modifiedIdentical)
	}
	if sameComparison.MigrationPath() != "No changes detected" {
		t.Errorf("expected 'No changes detected' for identical configs, got %q",
			sameComparison.MigrationPath())
	}
}
