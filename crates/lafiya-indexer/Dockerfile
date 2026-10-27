# Build from the repository root:
#   docker build -f crates/lafiya-indexer/Dockerfile -t lafiya-indexer .
#
# Hardened runtime image:
#   - Distroless cc-debian12:nonroot base (no shell, no package manager).
#   - Runs as the nonroot user (UID 65532) by default; the image is also
#     compatible with an explicit UID 10001 override via `--user 10001`.
#   - No writable paths: the container expects a read-only root filesystem
#     (e.g. `--read-only` / `readOnlyRootFilesystem: true`). The indexer must
#     not write to disk; any state belongs in an external store.
#   - No HEALTHCHECK: distroless has no shell or curl. Rely on the
#     orchestrator probe against the `/health` endpoint on port 8080.
#
# Build with locked dependencies (Cargo.lock is committed and pinned):
#   cargo build --release --locked -p lafiya-indexer

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p lafiya-indexer

FROM gcr.io/distroless/cc-debian12:nonroot
# CA certificates are provided by the distroless base image at
# /etc/ssl/certs/ca-certificates.crt for outbound TLS.
COPY --from=build /src/target/release/lafiya-indexer /usr/local/bin/lafiya-indexer
# Run as nonroot (UID 65532 in distroless). Override with `--user 10001` if a
# specific UID is required by policy.
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/lafiya-indexer"]
CMD ["run"]