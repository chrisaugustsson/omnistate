FROM rust:1.88-bookworm AS build
# rust-toolchain.toml pins the same version and asks for the WebAssembly target;
# naming the installed toolchain keeps rustup from fetching anything.
ENV RUSTUP_TOOLCHAIN=1.88.0
RUN apt-get update && apt-get install -y --no-install-recommends cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p omni-api && cp target/release/omni-api /omni-api

FROM debian:bookworm-slim
COPY --from=build /omni-api /usr/local/bin/omni-api
CMD ["omni-api"]
