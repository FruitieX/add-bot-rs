FROM rust:1.70@sha256:cf5513bb19a7a59fd271db14b4e71ed5d91df95c4cb1f431ac131231f2e6104a AS builder
WORKDIR /usr/src/add-bot
COPY . .
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse CARGO_TERM_COLOR=always
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:ed0ed67ed2620b48b94649605db557c3aee537893a749de4b9083934cbf6d5c6
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
