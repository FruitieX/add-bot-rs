FROM gcr.io/distroless/static@sha256:b033683de7de51d8cce5aa4b47c1b9906786f6256017ca8b17b2551947fcf6d8
COPY target/x86_64-unknown-linux-musl/release/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
