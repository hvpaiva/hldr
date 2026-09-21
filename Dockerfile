FROM rust:1-bookworm AS builder
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml crates/core/Cargo.toml
COPY crates/server/Cargo.toml crates/server/Cargo.toml
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml
RUN mkdir -p crates/core/src crates/server/src crates/cli/src \
    && echo "pub fn _dummy() {}" > crates/core/src/lib.rs \
    && echo "fn main() {}" > crates/server/src/main.rs \
    && echo "fn main() {}" > crates/cli/src/main.rs \
    && cargo build --release --locked -p hldr-server \
    && rm -rf crates

COPY crates crates
COPY migrations migrations
# Declared after the dependency layer so a new version does not rebuild it.
ARG HLDR_VERSION=dev
ARG HLDR_REVISION=unknown
# COPY preserves mtimes; cargo then keeps the dummy `fn main()` binary (~400KB).
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release --locked -p hldr-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 hldr \
    && useradd --uid 1000 --gid 1000 --home-dir /app --no-create-home hldr \
    && mkdir -p /var/lib/hldr \
    && chown hldr:hldr /var/lib/hldr

COPY --from=builder /src/target/release/hldr-server /usr/local/bin/hldr-server

USER 1000:1000
ENV HLDR_ADDR=0.0.0.0:8080 \
    HLDR_DATABASE=/var/lib/hldr/hldr.db \
    HLDR_LOG=info
EXPOSE 8080
CMD ["hldr-server"]
