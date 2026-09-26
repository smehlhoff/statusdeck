#!/usr/bin/env sh

set -eu

backup_path=${1:-statusdeck-backup.dump}

case "$backup_path" in
  /*) ;;
  *) backup_path="$(pwd)/$backup_path" ;;
esac

umask 077

temporary_path=$(mktemp "${backup_path}.tmp.XXXXXX")
trap 'rm -f "$temporary_path"' EXIT HUP INT TERM

docker compose \
  --env-file .env/.env \
  --file compose_base.yml \
  --file compose_prod.yml \
  exec -T postgres \
  pg_dump \
  --username=statusdeck \
  --format=custom \
  --no-owner \
  statusdeck > "$temporary_path"

mv "$temporary_path" "$backup_path"
trap - EXIT HUP INT TERM

printf '%s\n' "Wrote database backup to $backup_path"
