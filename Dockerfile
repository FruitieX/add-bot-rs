FROM rust:1.69@sha256:c2dcc19371a976065950915307b03a9c5514e5143bdd833635d5501a0642992e AS builder
WORKDIR /usr/src/add-bot
COPY . .
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse CARGO_TERM_COLOR=always
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:7606bef5684b393434f06a50a3d1a09808fee5a0240d37da5d181b1b121e7637
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
