## MissionControl — developer workflow
##
## Dev loop:
##   make env          # first time: create .env.dev from example
##   make dev          # start API + backing services (hot-reload)
##   make web          # optional: start Vite frontend dev server (:5173)
##   make mc-build     # build mc Rust binary locally
##
## Prod deploy:
##   make build        # build prod Docker image
##   make push         # push to ghcr.io
##   (ArgoCD picks up the new image and rolls it out to K8s)

COMPOSE_DEV  := docker compose -f docker-compose.dev.yml
COMPOSE_PROD := docker compose

IMAGE   ?= ghcr.io/ryanmerlin/missioncontrol
TAG     ?= $(shell git rev-parse --short HEAD)
VENV    ?= $(PWD)/.venv

.DEFAULT_GOAL := help

.PHONY: help env dev dev-down dev-logs dev-restart web \
        test test-client test-all \
        mc-build mc-install \
        build push \
        migrate lint \
        clean

help:  ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) \
	  | awk 'BEGIN {FS = ":.*##"}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

# ── Environment ───────────────────────────────────────────────────────────────

env:  ## Create .env.dev from example (skips if already exists)
	@[ -f .env.dev ] && echo ".env.dev already exists" || (cp .env.dev.example .env.dev && echo "Created .env.dev — edit it to add real API keys")

# ── Dev environment ───────────────────────────────────────────────────────────

dev: env  ## Start dev API + backing services with hot-reload
	$(COMPOSE_DEV) up --build -d
	@echo ""
	@echo "  API:      http://localhost:8008"
	@echo "  RustFS:   http://localhost:9000  (key: missioncontrol / missioncontrol-secret)"
	@echo "  Frontend: run 'make web' in a separate terminal"
	@echo ""
	@echo "Logs: make dev-logs"

dev-down:  ## Stop dev environment
	$(COMPOSE_DEV) down

dev-logs:  ## Follow dev logs
	$(COMPOSE_DEV) logs -f

dev-restart:  ## Restart dev API container (picks up Python changes without --reload missing them)
	$(COMPOSE_DEV) restart api

web:  ## Start Vite frontend dev server (proxies API to localhost:8008)
	cd web && npm install && npm run dev

# ── Tests ─────────────────────────────────────────────────────────────────────

test:  ## Run backend unit tests
	cd backend && UV_PROJECT_ENVIRONMENT=$(VENV) uv run python -m unittest discover -s tests -v

test-client:  ## Run MCP client tests
	cd distribution/mc-integration/missioncontrol-mcp && PYTHONPATH=src python -m unittest discover -v

test-all: test test-client  ## Run all tests

# ── mc Rust binary ────────────────────────────────────────────────────────────

mc-build:  ## Build mc binary (debug, fast)
	cargo build --manifest-path integrations/mc/Cargo.toml
	@echo "Binary: integrations/mc/target/debug/mc"

mc-build-release:  ## Build mc binary (release, optimized)
	cargo build --release --manifest-path integrations/mc/Cargo.toml
	@echo "Binary: integrations/mc/target/release/mc"

mc-install: mc-build-release  ## Install mc release binary to ~/.local/bin/mc
	install -m 755 integrations/mc/target/release/mc ~/.local/bin/mc
	@echo "Installed mc to ~/.local/bin/mc"

# ── Production image ──────────────────────────────────────────────────────────

build:  ## Build prod Docker image (tag: IMAGE:TAG and IMAGE:latest)
	docker build \
	  --target prod \
	  -t $(IMAGE):$(TAG) \
	  -t $(IMAGE):latest \
	  -f backend/Dockerfile .

push: build  ## Push prod image to ghcr.io
	docker push $(IMAGE):$(TAG)
	docker push $(IMAGE):latest
	@echo "Pushed $(IMAGE):$(TAG) — update gitops values.yaml to deploy"

# ── Database ──────────────────────────────────────────────────────────────────

migrate:  ## Run Alembic migrations against DATABASE_URL
	cd backend && UV_PROJECT_ENVIRONMENT=$(VENV) uv run alembic upgrade head

# ── Lint ──────────────────────────────────────────────────────────────────────

lint:  ## Run ruff linter on backend
	cd backend && UV_PROJECT_ENVIRONMENT=$(VENV) uv run ruff check app tests

# ── Cleanup ───────────────────────────────────────────────────────────────────

clean:  ## Remove dev Docker volumes (destroys local DB and object storage)
	$(COMPOSE_DEV) down -v
	@echo "Dev volumes removed"
