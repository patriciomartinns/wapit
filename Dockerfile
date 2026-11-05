# syntax=docker/dockerfile:1
FROM rust:1.99-alpine@sha256:27cd9befd4a05d6f6eb407c3e28365f5092f008bfc5946207cca76ae5179ef23 AS build
RUN apk add --no-cache musl-dev ca-certificates
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY public ./public
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/wapit /wapit
RUN mkdir -p /out/data && chown 1000:1000 /out/data

FROM scratch
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /wapit /wapit
COPY --from=build --chown=1000:1000 /out/data /data
ENV WAPIT_CONFIG_DIR=/app/config \
    WAPIT_DATA_DIR=/data \
    WAPIT_HOST=0.0.0.0
VOLUME ["/data"]
EXPOSE 8026 8027
USER 1000:1000
HEALTHCHECK --interval=10s --timeout=3s --retries=5 CMD ["/wapit", "healthcheck"]
ENTRYPOINT ["/wapit"]
