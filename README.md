# Bifrost Extensions

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/KooshaPari/argis-extensions/ci.yml?branch=main)](https://github.com/KooshaPari/argis-extensions/actions)
[![Go](https://img.shields.io/badge/go-1.22%2B-00ADD8?logo=go)](https://go.dev/)
[![AI Slop Inside](https://sladge.net/badge.svg)](https://sladge.net)

**Status:** active

Bifrost Extensions is a clean extension layer for the Bifrost LLM gateway, consuming upstream repositories as Go modules without modifications.

## Quick Start

```bash
# Build CLI
make cli-build

# Install CLI
make cli-install

# Initialize project
bifrost init

# Start server
bifrost server

# Deploy to Fly.io
bifrost deploy fly
```

## Documentation

- **[docs/README.md](docs/README.md)** - Main documentation index
- **[docs/INDEX.md](docs/INDEX.md)** - Complete file navigation
- **[docs/architecture/](docs/architecture/)** - Architecture & design principles
- **[docs/cli/](docs/cli/)** - CLI usage and integration
- **[docs/deployment/](docs/deployment/)** - Deployment guides
- **[docs/evaluation/](docs/evaluation/)** - Gap analysis and roadmap
- **[docs/guides/](docs/guides/)** - How-to guides and examples

## Architecture

This project follows a **clean extension layer pattern**:

- ✅ Consumes `bifrost` and `cliproxy` as Go modules
- ✅ Zero modifications to upstream repositories
- ✅ Easy to stay in sync with main developers
- ✅ Plugin-based extensibility

See [docs/architecture/PRINCIPLES.md](docs/architecture/PRINCIPLES.md) for details.

## Key Features

- **CLI Framework**: Cobra-based command-line interface
- **Serverless Deployment**: Fly.io, Vercel, Railway, Render, Homebox
- **Plugin System**: Extensible plugin architecture
- **Configuration**: Viper-based YAML + environment variables
- **Database**: PostgreSQL with migrations
- **Caching**: Redis support
- **Observability**: Structured logging and metrics

## Project Structure

```
bifrost-extensions/
├── README.md                 # This file
├── docs/                     # Documentation tree
│   ├── README.md            # Main docs index
│   ├── INDEX.md             # File navigation
│   ├── architecture/        # Design & principles
│   ├── cli/                 # CLI documentation
│   ├── deployment/          # Deployment guides
│   ├── evaluation/          # Gap analysis
│   ├── guides/              # How-to guides
│   └── reference/           # Reference materials
├── cmd/                     # CLI commands
├── api/                     # API routes
├── services/                # Business logic
├── config/                  # Configuration
├── db/                      # Database
└── plugins/                 # Plugin implementations
```

## Installation

### From Source

```bash
# Clone and setup
git clone https://github.com/KooshaPari/bifrost-extensions.git
cd bifrost-extensions

# Install dependencies
go mod download
go mod tidy

# Build CLI
make cli-build

# Install CLI
make cli-install

# Verify installation
bifrost --version
```

### Using Homebrew

```bash
brew install kooshapari/phenotype/bifrost
```

### Docker

```bash
docker pull ghcr.io/kooshapari/bifrost-extensions:latest
docker run bifrost --help
```

## Configuration

Bifrost uses environment variables and YAML configuration files:

```yaml
# bifrost.yml
api:
  host: localhost
  port: 8080
  tls: false

database:
  url: postgres://user:pass@localhost/bifrost
  max_connections: 10

cache:
  redis_url: redis://localhost:6379
  ttl: 3600

upstream:
  bifrost_url: https://bifrost.example.com
  cliproxy_url: https://cliproxy.example.com
```

Environment variables:
```bash
export BIFROST_API_HOST=localhost
export BIFROST_API_PORT=8080
export DATABASE_URL=postgres://...
export REDIS_URL=redis://...
```

## Development

```bash
# Setup development environment
make dev-setup

# Run tests
make test
make test-integration
make test-e2e

# Code quality
make lint
make fmt
make vet

# Build and run locally
make build
./bin/bifrost server

# Run with hot-reload
make dev-watch
```

## Testing

See [docs/guides/TESTING.md](docs/guides/TESTING.md) for comprehensive testing procedures.

```bash
# Run all tests
make test

# Run specific test package
make test-pkg pkg=api

# Test with coverage
make test-coverage

# View coverage report
open coverage.html
```

## Deployment

### Local Development

```bash
bifrost init
bifrost server --dev
```

### Docker

```bash
docker-compose up -d
bifrost migrate up
bifrost server
```

### Fly.io

```bash
bifrost deploy fly
```

### Other Platforms

- **Vercel** — Serverless deployment
- **Railway** — Simple PaaS deployment
- **Render** — Auto-scaling platform
- **Homebox** — Self-hosted option

See [docs/deployment/](docs/deployment/) for platform-specific guides.

## Plugin System

Bifrost Extensions uses a plugin architecture for extensibility:

```go
// Implement the Plugin interface
type MyPlugin struct{}

func (p *MyPlugin) Name() string {
    return "my-plugin"
}

func (p *MyPlugin) Init(ctx context.Context) error {
    // Plugin initialization
    return nil
}

func (p *MyPlugin) Execute(ctx context.Context, req *Request) (*Response, error) {
    // Plugin logic
    return &Response{}, nil
}

// Register plugin
registry.Register(&MyPlugin{})
```

See [docs/guides/](docs/guides/) for plugin development examples.

## Performance

Bifrost Extensions is optimized for:

- **Low Latency** — <100ms response time for typical requests
- **High Throughput** — 1000+ requests/second with caching
- **Scalability** — Horizontal scaling via load balancing
- **Resource Efficiency** — Minimal CPU and memory footprint

Benchmark results available in [docs/evaluation/PERFORMANCE.md](docs/evaluation/PERFORMANCE.md).

## Architecture Principles

Bifrost Extensions follows a **clean extension layer pattern**:

- ✅ **Composable**: Consume upstream libraries as Go modules
- ✅ **Maintainable**: Zero modifications to upstream code
- ✅ **Upgradeable**: Stay synchronized with main projects easily
- ✅ **Pluggable**: Extensible plugin architecture
- ✅ **Auditable**: Complete audit trail for changes and deployments

See [docs/architecture/PRINCIPLES.md](docs/architecture/PRINCIPLES.md) for detailed design rationale.

## Governance

- **Status**: Active
- **Language**: Go 1.21+
- **Type**: API Gateway & Extension Layer
- **Part of**: Bifrost + Cliproxy ecosystem
- **Testing**: All code requires unit + integration test coverage
- **Quality Gate**: No modifications to upstream repositories

## API Documentation

Bifrost Extensions provides OpenAPI-compatible API endpoints:

- `GET /v1/health` — Health check
- `POST /v1/process` — Process request through Bifrost
- `GET /v1/status` — System status and metrics
- `POST /v1/admin/config` — Update configuration

Full API reference: [docs/cli/API.md](docs/cli/API.md)

## Development Guide

```bash
# Code structure
cmd/                 # CLI entry points
api/                 # HTTP API handlers
services/            # Business logic
config/              # Configuration management
db/                  # Database models and migrations
plugins/             # Plugin implementations
tests/               # Test suite

# Adding a new endpoint
# 1. Define handler in api/routes/
# 2. Implement service logic in services/
# 3. Add database migration if needed
# 4. Write integration test
# 5. Document in docs/cli/
```

## Troubleshooting

**Port Already in Use**
```bash
lsof -i :8080  # Find process
kill -9 <PID>  # Terminate
```

**Database Connection Failed**
```bash
# Check PostgreSQL is running
psql $DATABASE_URL

# Run migrations
bifrost migrate up
```

**Plugin Load Error**
```bash
# Verify plugin path
bifrost --plugins-dir ./plugins

# Check logs
bifrost server --log-level debug
```

## Contributing

See [docs/contributing.md](docs/contributing.md) for contribution guidelines.

Pull requests are welcome! Please ensure:
- All tests pass: `make test`
- Code is formatted: `make fmt`
- Linting passes: `make lint`
- Documentation is updated

## License

See [LICENSE](LICENSE) file for details.

## Support

- **Issues**: GitHub Issues for bug reports
- **Discussions**: GitHub Discussions for questions
- **Documentation**: [docs/](docs/) folder
- **Examples**: [docs/guides/](docs/guides/) for recipes

## Related Projects

- **[Bifrost](https://github.com/bifrost-foundation/bifrost)** — LLM gateway (upstream)
- **[Cliproxy](https://github.com/cliproxy)** — Routing layer (upstream)
- **[Phenotype Ecosystem](https://projects.kooshapari.com)** — Related tools and libraries
