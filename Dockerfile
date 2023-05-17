FROM rust:1.69@sha256:ee5de9877e3df1180a2a95193ea954afcaac9c23d5dc3404cb987be5f2e432f8 AS builder
WORKDIR /usr/src/add-bot
COPY . .
RUN cargo install --path .

FROM debian:bullseye-slim@sha256:f4da3f9b18fc242b739807a0fb3e77747f644f2fb3f67f4403fafce2286b431a
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
