FROM rust:1.73@sha256:73af736ea21c14181c257bf674c7095a8bad6343a1eadd327a8bf1ce1c5209b4 AS builder
WORKDIR /usr/src/add-bot
COPY . .
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse CARGO_TERM_COLOR=always
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:19664a5752dddba7f59bb460410a0e1887af346e21877fa7cec78bfd3cb77da5
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
