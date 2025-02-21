# Huginn

A chat monitoring and data collection system built with Rust and TypeScript. This project is currently under active development and should not be used.

## Project Structure

```
├── apps/
│   ├── collector/         # Rust-based data collection services
│   │   ├── collector_service/  # Service for collecting data
│   │   ├── monitor_service/    # Service for monitoring streams
│   │   ├── writer_service/     # Service for writing to database
│   │   ├── common/            # Shared Rust code
│   │   ├── entity/           # Database entities
│   │   └── migration/        # Database migrations
│   └── kick/              # TypeScript-based Kick.com integration service
├── packages/
│   ├── eslint-config/    # Shared ESLint configurations
│   └── typescript-config/ # Shared TypeScript configurations
```
## Features

- High-performance data collection with Rust
- Multi-service architecture with:
  - Monitor service for stream status tracking
  - Collector service for real-time data gathering
  - Writer service for database operations
  - Native Kick.com API integration service
- Asynchronous processing using Tokio
- Message queuing with RabbitMQ
- Caching with Redis
- TimescaleDB for time-series data storage
- Structured logging with tracing
- Configuration management with environment variables
- Monorepo setup with Turborepo
- TypeScript/JavaScript tooling integration

## Prerequisites

- Rust (>= 1.85.0)
- Node.js (>=23.8.0)
- pnpm (>=10.4.0)
- Docker and Docker Compose

## Installation

1. Install dependencies:

```bash
pnpm install
```

2. Create a `.env` file in the root directory:
```env
# Main application configuration
APP_DATABASE_URL=postgres://huginn:huginn_password@localhost:5433/huginn
APP_RABBITMQ_URL=amqp://guest:guest@localhost:5672
APP_REDIS_URL=redis://localhost:6379
APP_SERVER_PORT=8080
APP_LOG_LEVEL=info
APP_HEALTH_CHECK_INTERVAL=300
APP_HEARTBEAT_INTERVAL=15
APP_RECOVERY_ATTEMPTS=3
APP_TOTAL_PARTITIONS=1
APP_PARTITION_ID=0
```

3. Configure service-specific environment variables:

Each service may require its own environment configuration. For example, the Kick service needs its own `.env` file in `apps/kick/.env`:
```env
# Proxy configuration for Kick service
PROXY_LIST="your_proxy_list_here"
SERVER_IPS="your_server_ips"
```

## Development

Start all services in development mode:
```bash
pnpm dev
```

Or start individual services:
```bash
# Monitor service only
pnpm composer:dev

# Collector service only
pnpm collector:dev

# Kick service only
pnpm kick:dev
```

### Available Scripts

- `pnpm dev` - Start all services in development mode
- `pnpm build` - Build all packages and applications
- `pnpm lint` - Run linting across the project
- `pnpm format` - Format code across the project
- `pnpm clean` - Clean build artifacts
- `pnpm composer:up` - Start all services in production mode
- `pnpm composer:down` - Stop all services

## Contributing

1. Ensure you have the correct Rust toolchain installed:
```bash
rustup override set nightly
rustup component add rustfmt clippy
```

2. Install development dependencies
3. Make your changes
4. Run formatting and linting before committing:
```bash
pnpm format
pnpm lint
```

## License

MIT
