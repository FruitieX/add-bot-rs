FROM rust:1.70@sha256:28ee8822965a932e229599b59928f8c2655b2a198af30568acf63e8aff0e8a3a AS builder
WORKDIR /usr/src/add-bot
COPY . .
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse CARGO_TERM_COLOR=always
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:3460d74bec6b88496cd183d7731930be55234c094f581f7dbdd96f56c1fc34d8
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
