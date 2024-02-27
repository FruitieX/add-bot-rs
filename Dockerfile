FROM gcr.io/distroless/static@sha256:9235ad98ee7b70ffee7805069ba0121b787eb1afbd104f714c733a8da18f9792
COPY target/x86_64-unknown-linux-musl/release/add-bot /usr/local/bin/add-bot
CMD ["add-bot"]
