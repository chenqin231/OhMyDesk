FROM node:24-bookworm-slim AS web-builder
WORKDIR /app/src/admin-web

RUN corepack enable && corepack prepare pnpm@11.8.0 --activate
COPY src/admin-web/package.json src/admin-web/pnpm-lock.yaml src/admin-web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile

COPY src/admin-web ./
RUN pnpm build

FROM rust:1-bookworm AS server-builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY scripts ./scripts
COPY src ./src
RUN cargo build -p server --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /app --shell /usr/sbin/nologin ohmydesk \
    && mkdir -p /app/data

COPY --from=server-builder /app/target/release/server /app/server
COPY --from=web-builder /app/src/admin-web/dist /app/web

RUN chown -R ohmydesk:ohmydesk /app
USER ohmydesk

ENV OHMYDESK_WEB_DIR=/app/web
ENV DATABASE_URL=sqlite:/app/data/ohmydesk.db
EXPOSE 8765
VOLUME ["/app/data"]

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
  CMD wget -qO- http://127.0.0.1:8765/ >/dev/null || exit 1

CMD ["/app/server"]
