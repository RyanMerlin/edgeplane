## MissionControl — developer workflow
##
## Dev loop:
##   make dev          # start mc-controlplane + postgres + web (docker-compose.mc-dev.yml)
##   make mc-build     # build mc Rust binary locally
##
## Prod deploy:
##   make build        # build prod Docker image
##   make push         # push to ghcr.io
##   (ArgoCD picks up the new image and rolls it out to K8s)

COMPOSE_DEV  := docker compose -f docker-compose.mc-dev.yml
COMPOSE_PROD := docker compose

IMAGE   ?= ghcr.io/ryanmerlin/missioncontrol
TAG     ?= $(shell git rev-parse --short HEAD)

.DEFAULT_GOAL := help

.PHONY: help dev dev-down dev-logs dev-restart web \
        test test-client test-all \
        mc-build mc-build-release mc-install \
        build push \
        migrate lint \
        clean

help:  ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) \
	  | awk 'BEGIN {FS = ":.*##"}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

# ── Dev environment ───────────────────────────────────────────────────────────

dev:  ## Start dev stack (mc-controlplane + postgres + web)
	$(COMPOSE_DEV) up --build -d
	@echo ""
	@echo "  API:      http://localhost:8008"
	@echo "  Frontend: http://localhost:5173"
	@echo ""
	@echo "Logs: make dev-logs"

dev-down:  ## Stop dev environment
	$(COMPOSE_DEV) down

dev-logs:  ## Follow dev logs
	$(COMPOSE_DEV) logs -f

dev-restart:  ## Restart mc-controlplane container
	$(COMPOSE_DEV) restart mc-controlplane

web:  ## Start Vite frontend dev server (proxies API to localhost:8008)
	cd web && npm install && npm run dev

# ── Tests ─────────────────────────────────────────────────────────────────────

test:  ## Run mc-controlplane unit tests
	cargo test --manifest-path crates/mc-controlplane/Cargo.toml

test-client:  ## Run MCP integration client tests
	cd distribution/mc-integration/missioncontrol-mcp && PYTHONPATH=src python -m unittest discover -v

test-all: test test-client  ## Run all tests

# ── mc Rust binary ────────────────────────────────────────────────────────────

mc-build:  ## Build mc binary (debug, fast)
	cargo build --manifest-path crates/mc/Cargo.toml
	@echo "Binary: crates/mc/target/debug/mc"

mc-build-release:  ## Build mc binary (release, optimized)
	cargo build --release --manifest-path crates/mc/Cargo.toml
	@echo "Binary: crates/mc/target/release/mc"

mc-install: mc-build-release  ## Install mc release binary to ~/.local/bin/mc
	install -m 755 crates/mc/target/release/mc ~/.local/bin/mc
	@echo "Installed mc to ~/.local/bin/mc"

# ── Production image ──────────────────────────────────────────────────────────

build:  ## Build prod Docker image (tag: IMAGE:TAG and IMAGE:latest)
	docker build \
	  -t $(IMAGE):$(TAG) \
	  -t $(IMAGE):latest \
	  -f crates/mc-controlplane/Dockerfile .

push: build  ## Push prod image to ghcr.io
	docker push $(IMAGE):$(TAG)
	docker push $(IMAGE):latest
	@echo "Pushed $(IMAGE):$(TAG) — update gitops values.yaml to deploy"

# ── Database ──────────────────────────────────────────────────────────────────

migrate:  ## Run SQLx migrations (requires DATABASE_URL)
	cargo sqlx migrate run --manifest-path crates/mc-controlplane/Cargo.toml \
	  --source crates/mc-controlplane/migrations

# ── Lint ──────────────────────────────────────────────────────────────────────

lint:  ## Run cargo clippy on mc-controlplane
	cargo clippy --manifest-path crates/mc-controlplane/Cargo.toml -- -D warnings

# ── Cleanup ───────────────────────────────────────────────────────────────────

clean:  ## Remove dev Docker volumes (destroys local DB and object storage)
	$(COMPOSE_DEV) down -v
	@echo "Dev volumes removed"
