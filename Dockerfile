FROM rust:1.89.0-bookworm AS builder

WORKDIR /usr/src/lord

COPY . .

RUN cargo build --bin lord --release

FROM debian:bookworm-slim

COPY --from=builder /usr/src/lord/target/release/lord /usr/local/bin
RUN apt-get update && apt-get install -y openssl

ENV RUST_BACKTRACE=1
ENV RUST_LOG=info
