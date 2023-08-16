FROM rust:1.71@sha256:69dcbc9d7029910a087b99717cd1779887bf69a0f695b22e3663b0d71d786c27 AS builder
WORKDIR /usr/src/add-bot
COPY . .
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse CARGO_TERM_COLOR=always
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:61386e11b5256efa33823cbfafd668dd651dbce810b24a8fb7b2e32fa7f65a85
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
