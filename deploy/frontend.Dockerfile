FROM node:24-bookworm@sha256:be23f54a88d34e8824c741b19b91064094f92c1c97b194144bfc8b50d67258e2 AS build

WORKDIR /src

COPY frontend/package*.json ./

RUN npm ci

COPY frontend/ ./

RUN npm run build

FROM nginxinc/nginx-unprivileged:1.30.4-alpine@sha256:9b87ad3dd9f431c733f19dfb278c7eb3dba9dca381942c79818bb42f1a566a83

ARG STATUSDECK_VERSION=0.1.0
ARG STATUSDECK_REVISION=unknown

LABEL org.opencontainers.image.title="StatusDeck frontend" \
    org.opencontainers.image.version=$STATUSDECK_VERSION \
    org.opencontainers.image.revision=$STATUSDECK_REVISION \
    org.opencontainers.image.licenses="MIT"

COPY deploy/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /src/dist /usr/share/nginx/html

USER 101
