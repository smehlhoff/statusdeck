SHELL := /bin/bash
.ONESHELL:

ENV_FILE ?= .env/.env
COMPOSE := docker compose --env-file $(ENV_FILE) -f compose_base.yml -f compose_dev.yml

.PHONY: dev infra-up infra-down migrate api worker frontend docker reset

dev: infra-up migrate
	$(COMPOSE) up --build --no-deps backend worker frontend

infra-up:
	$(COMPOSE) up -d --wait postgres

infra-down:
	$(COMPOSE) down

migrate:
	$(COMPOSE) run --rm --build migrate

api:
	$(COMPOSE) up --build backend

worker:
	$(COMPOSE) up --build worker

frontend:
	$(COMPOSE) up --build frontend worker

docker:
	$(COMPOSE) up -d --build --wait

reset:
	./scripts/docker-reset.sh
