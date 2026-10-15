# The integration test container: it runs inside the Compose network, so the
# tests reach the proxy, Postgres and RustFS by their service names. The
# repository is mounted at /workspace (docker-compose.yml) and built here.
FROM rust:1.88-bookworm
ENV RUSTUP_TOOLCHAIN=1.88.0 CARGO_TARGET_DIR=/cargo-target
WORKDIR /workspace
CMD ["cargo", "test", "-p", "omni-core", "--features", "stack-tests", "--test", "stack"]
