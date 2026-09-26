FROM rust:1.97-bookworm@sha256:14bc9c5966e7b3a385794b3d5389a8765668342025fbcc7b2e3d2866ac4bd8c3 AS build

WORKDIR /src

COPY backend/Cargo.toml backend/Cargo.lock* backend/
COPY backend/src backend/src
COPY backend/migrations backend/migrations

RUN cargo build --manifest-path backend/Cargo.toml --release --locked

FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241

ARG STATUSDECK_VERSION=0.1.0
ARG STATUSDECK_REVISION=unknown

LABEL org.opencontainers.image.title="StatusDeck" \
    org.opencontainers.image.version=$STATUSDECK_VERSION \
    org.opencontainers.image.revision=$STATUSDECK_REVISION \
    org.opencontainers.image.licenses="MIT"

RUN apt-get update \
    && apt-get install --no-install-recommends -y \
        ca-certificates \
        wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home statusdeck

COPY --from=build /src/backend/target/release/statusdeck-backend /usr/local/bin/statusdeck

USER 10001

ENTRYPOINT ["/usr/local/bin/statusdeck"]
