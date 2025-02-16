# Huginn

A modern monitoring and data collection system built with Rust and TypeScript.

## Project Structure

.
├── apps/
│   └── collector/         # Rust-based data collector service
├── packages/
│   ├── eslint-config/    # Shared ESLint configurations
│   └── typescript-config/ # Shared TypeScript configurations

## Features

- High-performance data collection with Rust
- Asynchronous processing using Tokio
- Structured logging with slog
- Configuration management with environment variables
- Monorepo setup with Turborepo
- TypeScript/JavaScript tooling integration

## Getting Started

### Prerequisites

- Rust (nightly)
- Node.js
- pnpm
- Docker (for development database)

### Installation

1. Install dependencies:

```bash
pnpm install
```

2. Set up the development database:
```bash
pnpm db:up
```

3. Create a `.env` file in the collector app directory with required environment variables:
```env
APP_DATABASE_URL=postgres://localhost:5432/huginn
```

### Development

Start the collector service in development mode:
```bash
pnpm collector:dev
```

### Available Scripts

- `pnpm build` - Build all packages and applications
- `pnpm lint` - Run linting across the project
- `pnpm format` - Format code across the project
- `pnpm clean` - Clean build artifacts
- `pnpm db:up` - Start the development database
- `pnpm db:down` - Stop the development database

## Contributing

1. Ensure you have the correct Rust toolchain installed (nightly)
2. Install development dependencies
3. Make your changes
4. Run formatting and linting before committing:
```bash
pnpm format
pnpm lint
```

## License

MIT
