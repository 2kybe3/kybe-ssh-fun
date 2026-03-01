FROM rust:latest AS builder
WORKDIR /usr/src/kybe-ssh-fun

RUN rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.lock ./
COPY static ./static
COPY src ./src

RUN cargo build --release --target x86_64-unknown-linux-musl

FROM alpine:latest
RUN apk add --no-cache openssh

RUN adduser -D -s /usr/local/bin/kybe-ssh-fun kybe
RUN echo "kybe:" | chpasswd

RUN echo "PermitEmptyPasswords yes" >> /etc/ssh/sshd_config
RUN echo "Port 2222" >> /etc/ssh/sshd_config
RUN echo "HostKey /etc/ssh/ssh_host_ed25519_key" >> /etc/ssh/sshd_config

RUN > /etc/motd

COPY --from=builder /usr/src/kybe-ssh-fun/target/x86_64-unknown-linux-musl/release/kybe-ssh-fun /usr/local/bin/kybe-ssh-fun
RUN chmod +x /usr/local/bin/kybe-ssh-fun

EXPOSE 2222
CMD ["/usr/sbin/sshd", "-D"]
