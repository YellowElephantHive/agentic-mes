FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --locked --release --package mes-api

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mes-api /usr/local/bin/mes-api
ENV APP_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["mes-api"]
