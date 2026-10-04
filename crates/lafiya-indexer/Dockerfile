# Build from the repository root:
#   docker build -f crates/lafiya-indexer/Dockerfile -t lafiya-indexer .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p lafiya-indexer

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 indexer
COPY --from=build /src/target/release/lafiya-indexer /usr/local/bin/lafiya-indexer
USER indexer
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD curl -fsS http://localhost:8080/health || exit 1
ENTRYPOINT ["lafiya-indexer"]
CMD ["run"]
