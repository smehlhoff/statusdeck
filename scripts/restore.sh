#!/usr/bin/env sh

set -eu

if [ "$#" -ne 1 ]; then
  printf '%s\n' "Usage: $0 /path/to/statusdeck-backup.dump" >&2
  exit 2
fi

backup_path=$1

if [ ! -f "$backup_path" ]; then
  printf '%s\n' "Backup file does not exist: $backup_path" >&2
  exit 2
fi

compose() {
  docker compose --env-file .env/.env --file compose_base.yml --file compose_prod.yml "$@"
}

printf '%s\n' "This replaces the current StatusDeck database. The backup path is: $backup_path"
printf '%s' "Type RESTORE to continue: "
read -r confirmation

if [ "$confirmation" != "RESTORE" ]; then
  printf '%s\n' "Restore cancelled."
  exit 1
fi

# Read the archive completely before stopping the application.
compose exec -T postgres pg_restore --file=/dev/null < "$backup_path"

compose stop backend worker frontend

# Cleanup and restore must commit together so a failure preserves existing data.
compose exec -T postgres \
  pg_restore \
  --username=statusdeck \
  --dbname=statusdeck \
  --no-owner \
  --clean \
  --if-exists \
  --single-transaction \
  --exit-on-error < "$backup_path"

compose run --rm migrate
compose up --detach backend worker frontend

printf '%s\n' "Restore completed; verify /health/ready and sign in before accepting traffic."
