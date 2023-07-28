FROM rust:1.71@sha256:92d62e4b8d8a8fd3d59d89b5227d7f7718a6c126990e3c2eac03517f72e5a683 AS builder
WORKDIR /usr/src/add-bot
COPY . .
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse CARGO_TERM_COLOR=always
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:fd3b382990294beb46aa7549edb9f40b11a070f959365ef7f316724b2e425f90
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
