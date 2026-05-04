# Bifrost Extensions

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

## Development

See [docs/guides/TESTING.md](docs/guides/TESTING.md) for testing procedures.

## License

See LICENSE file for details.
