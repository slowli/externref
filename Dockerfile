# Docker image for the `externref` CLI executable.
# See the CLI crate readme for the usage instructions.

FROM clux/muslrust:stable AS builder
ADD . ./
ARG FEATURES=tracing
RUN cargo build --manifest-path=crates/cli/Cargo.toml --release \
  --no-default-features --features=$FEATURES \
  --target-dir /volume/target

FROM gcr.io/distroless/static-debian11
COPY --from=builder /volume/target/x86_64-unknown-linux-musl/release/externref /
CMD /externref
