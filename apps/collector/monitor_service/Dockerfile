FROM rust:1.75 AS builder

WORKDIR /usr/src/app
# Copy the entire workspace
COPY . .

# Build the application
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/app/target/release/monitor_service /usr/local/bin/

CMD ["monitor_service"]