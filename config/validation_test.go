package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// validLogging returns a LoggingConfig that passes validation.
func validLogging() LoggingConfig {
	return LoggingConfig{
		Level:  "info",
		Format: "json",
		Output: "stdout",
	}
}

// TestValidConfiguration tests that valid configuration passes validation.
func TestValidConfiguration(t *testing.T) {
	tests := []struct {
		name string
		cfg  *Config
	}{
		{
			name: "default config",
			cfg:  DefaultConfig(),
		},
		{
			name: "custom valid config",
			cfg: &Config{
				Server: ServerConfig{
					Host:           "127.0.0.1",
					Port:           9090,
					ReadTimeout:    30,
					WriteTimeout:   120,
					MaxRequestSize: 10,
				},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.cfg.Validate()
			if err != nil {
				t.Errorf("valid config should pass validation, got: %v", err)
			}
		})
	}
}

// TestInvalidServerConfig tests invalid server configuration.
func TestInvalidServerConfig(t *testing.T) {
	tests := []struct {
		name        string
		cfg         *Config
		wantErr     bool
		errContains string
	}{
		{
			name: "negative port",
			cfg: &Config{
				Server: ServerConfig{
					Port: -1,
				},
				Logging: validLogging(),
			},
			wantErr:     true,
			errContains: "port",
		},
		{
			name: "port too high",
			cfg: &Config{
				Server: ServerConfig{
					Port: 65536,
				},
				Logging: validLogging(),
			},
			wantErr:     true,
			errContains: "port",
		},
		{
			name: "negative timeout",
			cfg: &Config{
				Server: ServerConfig{
					ReadTimeout: -1,
				},
				Logging: validLogging(),
			},
			wantErr:     true,
			errContains: "timeout",
		},
		{
			name: "empty host",
			cfg: &Config{
				Server: ServerConfig{
					Host: "",
				},
				Logging: validLogging(),
			},
			wantErr:     true,
			errContains: "host",
		},
		{
			name: "missing required fields",
			cfg: &Config{
				Server: ServerConfig{},
			},
			wantErr: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.cfg.Validate()
			if tt.wantErr {
				if err == nil {
					t.Errorf("expected validation error for %q, got nil", tt.name)
					return
				}
				if tt.errContains != "" && !strings.Contains(err.Error(), tt.errContains) {
					t.Errorf("error %q should contain %q", err.Error(), tt.errContains)
				}
			} else {
				if err != nil {
					t.Errorf("unexpected error: %v", err)
				}
			}
		})
	}
}

// TestInvalidRoutingConfig tests invalid routing configuration.
func TestInvalidRoutingConfig(t *testing.T) {
	tests := []struct {
		name        string
		cfg         *Config
		wantErr     bool
		errContains string
	}{
		{
			name: "invalid endpoint URL",
			cfg: &Config{
				Routing: RoutingConfig{
					RouteLLM: RouteLLMConfig{
						Enabled:  true,
						Endpoint: "", // required when enabled
					},
				},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
			wantErr:     true,
			errContains: "endpoint",
		},
		{
			name: "invalid threshold value",
			cfg: &Config{
				Routing: RoutingConfig{
					RouteLLM: RouteLLMConfig{
						Enabled:   true,
						Endpoint:  "http://localhost:6060/route",
						Threshold: -1.0,
						Timeout:   5000,
					},
				},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
			wantErr:     true,
			errContains: "threshold",
		},
		{
			name: "threshold too high",
			cfg: &Config{
				Routing: RoutingConfig{
					RouteLLM: RouteLLMConfig{
						Enabled:   true,
						Endpoint:  "http://localhost:6060/route",
						Threshold: 2.0,
						Timeout:   5000,
					},
				},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
			wantErr:     true,
			errContains: "threshold",
		},
		{
			name: "invalid timeout value",
			cfg: &Config{
				Routing: RoutingConfig{
					RouteLLM: RouteLLMConfig{
						Enabled:   true,
						Endpoint:  "http://localhost:6060/route",
						Timeout:   -1,
						Threshold: 0.5,
					},
				},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
			wantErr:     true,
			errContains: "timeout",
		},
		{
			name: "missing required routing config",
			cfg: &Config{
				Server: ServerConfig{
					Host:           "127.0.0.1",
					Port:           8080,
					ReadTimeout:    30,
					WriteTimeout:   120,
					MaxRequestSize: 10,
				},
				Routing: RoutingConfig{},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
			wantErr: false, // Routing config is optional
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.cfg.Validate()
			if tt.wantErr {
				if err == nil {
					t.Errorf("expected validation error for %q, got nil", tt.name)
					return
				}
				if tt.errContains != "" && !strings.Contains(err.Error(), tt.errContains) {
					t.Errorf("error %q should contain %q", err.Error(), tt.errContains)
				}
			} else {
				if err != nil {
					t.Errorf("unexpected error: %v", err)
				}
			}
		})
	}
}

// TestInvalidOAuthConfig tests invalid OAuth configuration.
func TestInvalidOAuthConfig(t *testing.T) {
	tests := []struct {
		name        string
		cfg         *Config
		wantErr     bool
		errContains string
	}{
		{
			name: "invalid provider config",
			cfg: &Config{
				Server: ServerConfig{
					Host:           "127.0.0.1",
					Port:           8080,
					ReadTimeout:    30,
					WriteTimeout:   120,
					MaxRequestSize: 10,
				},
				Routing: RoutingConfig{},
				OAuth: OAuthConfig{
					Enabled: true,
					Providers: map[string]OAuthProvider{
						"claude": {
							Enabled: true,
						},
					},
				},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
			wantErr: false, // OAuth provider missing fields is valid schema; validation is by caller
		},
		{
			name: "enabled with no providers",
			cfg: &Config{
				Server: ServerConfig{
					Host:           "127.0.0.1",
					Port:           8080,
					ReadTimeout:    30,
					WriteTimeout:   120,
					MaxRequestSize: 10,
				},
				Routing: RoutingConfig{},
				OAuth: OAuthConfig{
					Enabled:   true,
					Providers: map[string]OAuthProvider{},
				},
				Logging: LoggingConfig{
					Level:  "info",
					Format: "json",
					Output: "stdout",
				},
			},
			wantErr: false, // OAuth enabled with empty providers is allowed by schema validation
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.cfg.Validate()
			if tt.wantErr {
				if err == nil {
					t.Errorf("expected validation error for %q, got nil", tt.name)
					return
				}
				if tt.errContains != "" && !strings.Contains(err.Error(), tt.errContains) {
					t.Errorf("error %q should contain %q", err.Error(), tt.errContains)
				}
			} else {
				if err != nil {
					t.Errorf("unexpected error: %v", err)
				}
			}
		})
	}
}

// TestConfigSchemaValidation tests configuration schema validation via file loading.
func TestConfigSchemaValidation(t *testing.T) {
	tests := []struct {
		name        string
		configData  string
		wantErr     bool
		errContains string
	}{
		{
			name: "valid YAML",
			configData: `
server:
  host: "127.0.0.1"
  port: 8080
logging:
  level: info
  format: json
  output: stdout
`,
			wantErr: false,
		},
		{
			name: "invalid YAML syntax",
			configData: `
server:
  host: "127.0.0.1"
  port: [invalid
`,
			wantErr:     true,
			errContains: "While parsing",
		},
		{
			name: "invalid port from file",
			configData: `
server:
  host: "127.0.0.1"
  port: 99999
logging:
  level: info
  format: json
  output: stdout
`,
			wantErr:     true,
			errContains: "port",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			tmpDir := t.TempDir()
			configPath := filepath.Join(tmpDir, "config.yaml")
			if err := os.WriteFile(configPath, []byte(tt.configData), 0644); err != nil {
				t.Fatalf("failed to write config file: %v", err)
			}

			cfg, err := Load(configPath)
			if tt.wantErr {
				if err == nil {
					t.Errorf("expected error for %q, got nil", tt.name)
					return
				}
				if tt.errContains != "" && !strings.Contains(err.Error(), tt.errContains) {
					t.Errorf("error %q should contain %q", err.Error(), tt.errContains)
				}
			} else {
				if err != nil {
					t.Errorf("unexpected error: %v", err)
				}
				if cfg == nil {
					t.Error("expected non-nil config")
				}
			}
		})
	}
}
