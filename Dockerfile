FROM clux/muslrust:latest AS builder
WORKDIR /usr/src/kybe-ssh-fun

COPY Cargo.toml Cargo.lock ./
COPY static ./static
COPY src ./src

RUN cargo build --release --target x86_64-unknown-linux-musl

FROM alpine:latest
WORKDIR /opt/kybe-ssh-fun

COPY --from=builder /usr/src/kybe-ssh-fun/target/x86_64-unknown-linux-musl/release/kybe-ssh-fun /usr/local/bin/kybe-ssh-fun
RUN chmod +x /usr/local/bin/kybe-ssh-fun

EXPOSE 2222
CMD ["/usr/local/bin/kybe-ssh-fun"]
