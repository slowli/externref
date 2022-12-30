# Docker image for the `externref` CLI executable.
# See the CLI crate readme for the usage instructions.

FROM clux/muslrust:stable AS builder
ADD ../.. ./
ARG FEATURES=tracing
RUN --mount=type=cache,id=cargo-registry,target=/root/.cargo/registry \
  --mount=type=cache,id=artifacts,target=/volume/target \
  cargo build --manifest-path=crates/cli/Cargo.toml --release \
    --no-default-features --features=$FEATURES \
    --target-dir /volume/target && \
  # Move the resulting executable so it doesn't get unmounted together with the cache
  mv /volume/target/x86_64-unknown-linux-musl/release/externref /volume/externref

FROM gcr.io/distroless/static-debian11
COPY --from=builder /volume/externref /
ENTRYPOINT ["/externref"]
