# Agent Rules - bifrost-extensions

**This project is managed through AgilePlus.**

## Overview

Bifrost-Extensions is a comprehensive extension and plugin framework for the Bifrost platform within the Phenotype ecosystem. It provides modular extension capabilities, API integration hooks, and customizable workflows that enable developers to extend Bifrost functionality without modifying core systems.

### Purpose & Goals

- **Mission**: Enable seamless extension of Bifrost capabilities through a robust plugin architecture
- **Primary Goal**: Provide a secure, sandboxed environment for third-party and internal extensions
- **Secondary Goals**:
  - Support multiple extension languages (Go, Rust, WebAssembly)
  - Provide hot-reload capabilities for development
  - Enable marketplace distribution of extensions
  - Maintain backward compatibility across Bifrost versions

### Key Responsibilities

1. **Plugin System**: Dynamic loading and lifecycle management of extensions
2. **API Hooks**: Pre/post hooks for Bifrost core operations
3. **Sandboxing**: Secure execution environment for untrusted extensions
4. **Configuration**: Extension-specific configuration management
5. **Event System**: Pub/sub for inter-extension communication
6. **Versioning**: Extension versioning and compatibility checking

## Stack

### Primary Language & Runtime
- **Language**: Go 1.24+ (core), WebAssembly (extensions)
- **Runtime**: Native with aggressive generics adoption
- **Architecture**: Plugin host + WASM runtime

### Core Dependencies
```go
// Plugin System
github.com/hashicorp/go-plugin          // HashiCorp plugin system
github.com/tetratelabs/wazero           // WebAssembly runtime
github.com/bytecodealliance/wasmtime-go // Alternative WASM runtime

// RPC/Communication
google.golang.org/grpc
github.com/golang/protobuf

// Sandboxing
github.com/google/gvisor/pkg/sentry     // gVisor sandbox
github.com/seccomp/libseccomp-golang    // seccomp-bpf

// Configuration
github.com/spf13/viper
gopkg.in/yaml.v3

// Event System
github.com/nats-io/nats.go

// Utilities
github.com/fsnotify/fsnotify            // File watching
github.com/google/uuid
```

### Extension Formats
- **Native Go**: Compiled plugins using go-plugin
- **WebAssembly**: WASI-compliant WASM modules
- **gRPC Services**: External microservice extensions
- **JavaScript**: QuickJS/V8 embedded runtime

### Build & Development Tools
- **Task Runner**: Task (Taskfile.yml)
- **Linting**: golangci-lint
- **Testing**: gotestsum
- **Documentation**: OpenAPI specs for extension APIs

## Quick Start

### Prerequisites

```bash
# Go 1.24+
brew install go@1.24

# Task runner
brew install go-task/tap/go-task

# WASI SDK (for WASM extensions)
brew install wasi-sdk

# Additional tools
go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest
```

### Installation

```bash
# Clone the repository
cd /Users/kooshapari/CodeProjects/Phenotype/repos/bifrost-extensions

# Install dependencies
go mod download

# Build the core
task build

# Verify installation
bifrost-ext --version
```

### Development Environment Setup

```bash
# Copy environment configuration
cp .env.example .env

# Initialize extension directory
mkdir -p extensions
cd extensions

# Create sample extension
bifrost-ext scaffold --name my-extension --type wasm
```

### Running the Extension Host

```bash
# Development mode
task dev

# Load specific extensions
task dev -- --extensions ./extensions/*.wasm

# Production mode
task build:release
./bin/bifrost-ext host
```

### Verification

```bash
# Run all tests
task test

# Test with sample extensions
task test:integration

# Check code quality
task lint
task format:check

# Health check
bifrost-ext status
```

## Architecture

### System Design

Bifrost-Extensions implements a host-guest architecture with multiple extension types:

```
┌─────────────────────────────────────────────────────────────┐
│                    Bifrost Core Application                  │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │              Extension Host                           │  │
│  ├──────────────────────────────────────────────────────┤  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────┐   │  │
│  │  │   Loader     │  │   Runtime    │  │  Hooks   │   │  │
│  │  │              │  │   Manager    │  │ Registry │   │  │
│  │  └──────────────┘  └──────────────┘  └──────────┘   │  │
│  └──────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│                    Extension Sandboxes                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │  WASM        │  │  Go Plugin   │  │  gRPC        │     │
│  │  Runtime     │  │  Runtime     │  │  Client      │     │
│  │  (Wazero)    │  │  (go-plugin) │  │              │     │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘     │
└─────────┼─────────────────┼─────────────────┼─────────────┘
          │                 │                 │
          ▼                 ▼                 ▼
┌─────────────────────────────────────────────────────────────┐
│                    Extensions                                │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐     │
│  │  Auth    │ │ Metrics  │ │  Custom  │ │  Vendor  │     │
│  │Extension │ │Extension │ │Extension │ │Extension │     │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘     │
└─────────────────────────────────────────────────────────────┘
```

### Component Breakdown

#### 1. Extension Host
- **Loader**: Discovers and loads extensions
- **Runtime Manager**: Manages extension lifecycles
- **Hooks Registry**: Registers API hooks

#### 2. Extension Runtimes
- **WASM Runtime**: Wazero-based sandbox
- **Go Plugin**: HashiCorp go-plugin system
- **gRPC Client**: External service connector

#### 3. Sandboxing
- **WASI**: Capability-based security model
- **Seccomp**: System call filtering
- **Resource Limits**: CPU, memory constraints

### Extension Lifecycle

```
Discovery → Load → Validate → Initialize → Register Hooks → Running
                                              ↓
                                         Shutdown
                                              ↓
                                         Cleanup
```

### Hook System

```go
// Pre-hook example
host.RegisterPreHook("request.process", func(ctx context.Context, req *Request) error {
    // Extension can modify request
    req.Headers["X-Extended"] = "true"
    return nil
})

// Post-hook example
host.RegisterPostHook("response.send", func(ctx context.Context, resp *Response) error {
    // Extension can modify response
    resp.Headers["X-Processed-By"] = "extension-name"
    return nil
})
```

### Extension Manifest

```yaml
# extension.yaml
apiVersion: bifrost.extensions.phenotype.dev/v1
kind: Extension
metadata:
  name: my-extension
  version: 1.0.0
  author: Phenotype Team
  description: Example extension

spec:
  runtime: wasm  # wasm | goplugin | grpc
  
  permissions:
    - network:outbound
    - filesystem:read:/data
    - bifrost:hooks:request.process
  
  hooks:
    - name: request.process
      type: pre
      priority: 100
    - name: response.send
      type: post
      priority: 50
  
  config:
    schema: config-schema.json
    defaults:
      timeout: 30s
```

## Quality Standards

### Testing Requirements

#### Test Coverage
- **Minimum Coverage**: 80% for host, 70% for runtimes
- **Critical Paths**: 95% for sandbox boundaries
- **Measurement**: `go test -coverprofile` with CI

#### Test Categories
```bash
# Unit tests
task test:unit

# Integration tests with real extensions
task test:integration

# Security tests
task test:security

# WASM-specific tests
task test:wasm
```

#### Security Testing
- Sandbox escape attempts
- Resource exhaustion tests
- Permission bypass tests

### Code Quality

#### Go Standards
```bash
# Linting
golangci-lint run --config=.golangci.yml

# Formatting
go fmt ./...
gofumpt -l -w .

# Security scan
gosec ./...
```

#### WASM Standards
- WASI-compliant modules
- No host system access without explicit capabilities
- Limited memory usage

### Performance Benchmarks

| Metric | Target | Measurement |
|--------|--------|-------------|
| Extension load time | < 100ms | Cold start |
| Hook latency | < 1ms | Per-hook overhead |
| Memory overhead | < 50MB | Per-extension |
| Throughput impact | < 5% | With extensions loaded |

## Git Workflow

### Branch Strategy

```
main
  │
  ├── feature/hot-reload
  │   └── PR #56 → squash merge ──┐
  │                               │
  ├── feature/extension-marketplace│
  │   └── PR #57 → squash merge ──┤
  │                               │
  ├── fix/sandbox-escape           │
  │   └── PR #58 → squash merge ──┤
  │                               │
  └── hotfix/security-patch ────────┘
      └── PR #59 → merge commit
```

### Branch Naming

```
feature/<scope>-<description>
fix/<component>-<issue>
security/<vulnerability>
refactor/<scope>
docs/<topic>
chore/<maintenance>
hotfix/<critical>
```

### Commit Conventions

```
feat(wasm): add WASI preview2 support

Updates WASM runtime to support WASI preview2
with component model for better interoperability.

Closes #123

security(sandbox): enforce seccomp-bpf on Linux

Adds system call filtering to prevent sandbox escape
on Linux hosts. Matches gVisor policy.
```

### Pull Request Process

1. **Pre-PR Checklist**:
   ```bash
   task lint
   task test
   task test:security
   task format:check
   ```

2. **PR Requirements**:
   - Link to AgilePlus spec
   - Security review for sandbox changes
   - Performance benchmarks for runtime changes

3. **Review Requirements**:
   - 1 approval from extensions team
   - Security approval for sandbox changes
   - CI passes including security tests

4. **Merge Strategy**:
   - Squash merge for features
   - Regular merge for hotfixes

## File Structure

```
bifrost-extensions/
├── cmd/
│   └── bifrost-ext/           # CLI entry
│       └── main.go
│
├── pkg/
│   ├── host/                   # Extension host
│   │   ├── host.go
│   │   ├── loader.go
│   │   └── registry.go
│   ├── runtime/                # Runtime implementations
│   │   ├── wasm/
│   │   │   ├── runtime.go
│   │   │   └── sandbox.go
│   │   ├── goplugin/
│   │   │   └── runtime.go
│   │   └── grpc/
│   │       └── client.go
│   ├── hooks/                  # Hook system
│   │   ├── registry.go
│   │   ├── pre.go
│   │   └── post.go
│   ├── sandbox/                # Sandboxing
│   │   ├── seccomp.go
│   │   ├── capabilities.go
│   │   └── limits.go
│   ├── config/                 # Configuration
│   │   ├── parser.go
│   │   └── validator.go
│   └── api/                    # Extension API
│       └── types.go
│
├── examples/                   # Example extensions
│   ├── wasm-example/
│   ├── go-plugin-example/
│   └── grpc-example/
│
├── tests/
│   ├── integration/
│   └── fixtures/
│
├── docs/
│   ├── architecture.md
│   ├── writing-extensions.md
│   └── security.md
│
├── Taskfile.yml
├── go.mod
├── go.sum
├── README.md
├── CHANGELOG.md
└── AGENTS.md                   # This file
```

## CLI

### Core Commands

```bash
# Host Operations
bifrost-ext host               # Start extension host
bifrost-ext host --config ./config.yaml

# Extension Management
bifrost-ext list               # List loaded extensions
bifrost-ext load <path>        # Load extension
bifrost-ext unload <name>      # Unload extension
bifrost-ext reload <name>      # Hot reload extension

# Development
bifrost-ext scaffold <name>    # Create extension template
bifrost-ext validate <path>    # Validate extension manifest
bifrost-ext test <path>        # Test extension locally

# Debugging
bifrost-ext logs <name>        # View extension logs
bifrost-ext inspect <name>     # Inspect extension state
bifrost-ext debug <name>       # Attach debugger

# Marketplace (future)
bifrost-ext search <query>     # Search marketplace
bifrost-ext install <name>     # Install from marketplace
bifrost-ext publish <path>     # Publish to marketplace

# Diagnostics
bifrost-ext doctor             # Health check
bifrost-ext version            # Version info
```

### Configuration

```yaml
# config.yaml
host:
  listen: localhost:8080
  extensions_dir: ./extensions

runtime:
  wasm:
    max_memory: 128MB
    timeout: 30s
  goplugin:
    allowed_protocols: [grpc]
  grpc:
    timeout: 10s

sandbox:
  enabled: true
  seccomp: true
  capabilities:
    - network:outbound
    - filesystem:read:/data

logging:
  level: info
  format: json
```

## Troubleshooting

### Common Issues

#### Issue: Extension fails to load

**Symptoms:**
```
Error: failed to load extension: incompatible ABI
```

**Diagnosis:**
```bash
# Check extension manifest
bifrost-ext validate ./my-extension/extension.yaml

# Verify runtime compatibility
cat ./my-extension/extension.yaml | grep runtime

# Check host version
bifrost-ext version
```

**Resolution:**
```bash
# Recompile with correct target
# For WASM:
tinygo build -target=wasi -o extension.wasm .

# Update manifest version
# Or use compatible runtime version
```

---

#### Issue: Sandbox blocking legitimate operations

**Symptoms:**
```
Error: capability denied: network:outbound
```

**Resolution:**
```bash
# Update extension manifest to request capability
# extension.yaml
spec:
  permissions:
    - network:outbound

# Reload extension
bifrost-ext reload my-extension
```

---

#### Issue: High memory usage with WASM extensions

**Diagnosis:**
```bash
# Check extension memory usage
bifrost-ext inspect my-extension | grep memory

# Monitor host memory
top -p $(pgrep bifrost-ext)
```

**Resolution:**
```bash
# Reduce memory limit in config
# config.yaml
runtime:
  wasm:
    max_memory: 64MB  # Reduce from 128MB

# Restart host
bifrost-ext host restart
```

---

#### Issue: Hook not being called

**Diagnosis:**
```bash
# Check hook registration
bifrost-ext inspect my-extension | grep hooks

# Verify hook name matches
# Check hook priority order
bifrost-ext list --hooks
```

**Resolution:**
```bash
# Ensure correct hook name in manifest
# extension.yaml
spec:
  hooks:
    - name: request.process  # Must match core hook name
      type: pre
      priority: 100

# Reload after manifest change
bifrost-ext reload my-extension
```

---

### Debug Mode

```bash
# Enable debug logging
export BIFROST_EXT_LOG_LEVEL=debug

# Run with tracing
bifrost-ext host --trace

# Extension-specific debugging
bifrost-ext debug my-extension
```

### Security Audit

```bash
# Run security scan
task test:security

# Check for capability violations
bifrost-ext audit --capabilities

# Review sandbox logs
cat /var/log/bifrost-ext/sandbox.log
```

---

## Agent Self-Correction & Verification Protocols

### Critical Rules

1. **Sandbox Integrity**
   - Never relax sandbox for convenience
   - All capabilities must be explicit
   - Regular security audits

2. **API Stability**
   - Version hooks properly
   - Deprecation period for breaking changes
   - Migration guides for extensions

3. **Performance**
   - Benchmark hook overhead
   - Memory limits enforced
   - No blocking in hooks

4. **AgilePlus Integration**
   - Reference extension specs
   - Update for new hook points

---

*This AGENTS.md is a living document. Update it as bifrost-extensions evolves.*
