#!/usr/bin/env sh

set -eu

exec cargo run \
  --manifest-path backend/Cargo.toml \
  --locked \
  -- \
  validate-catalog
