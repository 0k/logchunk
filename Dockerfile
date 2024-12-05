FROM alpine:3.20

# Install dependencies

## sqlite-dev and sqlite-static are needed for compilation
RUN apk add --no-cache musl-dev openssl-dev sqlite-dev sqlite-static clang-dev
RUN apk add --no-cache cargo rustup

## add user with UID 1000 and GID 1000

RUN addgroup -g 1000 rust && \
    adduser -D -u 1000 -G rust rust

USER rust

RUN rustup-init -y

WORKDIR /home/rust/rs
