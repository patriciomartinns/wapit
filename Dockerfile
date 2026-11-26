# syntax=docker/dockerfile:1
FROM rust:1.100-alpine@sha256:8d0e82abfdf9a38071e07cabc5afba4ddbe077631a80e4e742a760c38beadc33 AS build
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
