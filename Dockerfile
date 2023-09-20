FROM rust:1.72@sha256:8a4ca3ca75afbc97bcf5362e9a694fe049d15734fbbaf82b8b7e224616c1254b AS builder
WORKDIR /usr/src/add-bot
COPY . .
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse CARGO_TERM_COLOR=always
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:40c0654c268cdc2077638b2d1660b7bf9824ae3e71dd59806f76efaeb9f56715
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
