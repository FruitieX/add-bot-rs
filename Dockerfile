FROM gcr.io/distroless/static
COPY target/x86_64-unknown-linux-musl/release/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
