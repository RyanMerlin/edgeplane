## Edgeplane — developer workflow
##
## Dev loop:
##   make dev          # start edgeplane-tower + postgres + web (docker-compose.edgeplane-dev.yml)
##   make edgeplane-build     # build edgeplane Rust binary locally
##
## Prod deploy:
##   make build        # build prod Docker image
##   make push         # push to ghcr.io
##   (ArgoCD picks up the new image and rolls it out to K8s)

COMPOSE_DEV  := docker compose -f docker-compose.edgeplane-dev.yml
COMPOSE_PROD := docker compose

IMAGE   ?= ghcr.io/ryanmerlin/edgeplane
TAG     ?= $(shell git rev-parse --short HEAD)

.DEFAULT_GOAL := help

.PHONY: help dev dev-down dev-logs dev-restart web \
        test test-client test-all \
        edgeplane-build edgeplane-build-release edgeplane-install \
        build push \
        migrate lint \
        clean

help:  ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) \
	  | awk 'BEGIN {FS = ":.*##"}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

# ── Dev environment ───────────────────────────────────────────────────────────

dev:  ## Start dev stack (edgeplane-tower + postgres + web)
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

dev-restart:  ## Restart edgeplane-tower container
	$(COMPOSE_DEV) restart edgeplane-tower

web:  ## Start Vite frontend dev server (proxies API to localhost:8008)
	cd web && npm install && npm run dev

# ── Tests ─────────────────────────────────────────────────────────────────────

test:  ## Run edgeplane-tower unit tests
	cargo test --manifest-path crates/edgeplane-tower/Cargo.toml

test-client:  ## Run MCP integration client tests
	cd distribution/edgeplane-integration/edgeplane-mcp && PYTHONPATH=src python -m unittest discover -v

test-all: test test-client  ## Run all tests

# ── edgeplane Rust binary ────────────────────────────────────────────────────────────

edgeplane-build:  ## Build edgeplane binary (debug, fast)
	cargo build --manifest-path crates/edgeplane/Cargo.toml
	@echo "Binary: crates/edgeplane/target/debug/edgeplane"

edgeplane-build-release:  ## Build edgeplane binary (release, optimized)
	cargo build --release --manifest-path crates/edgeplane/Cargo.toml
	@echo "Binary: crates/edgeplane/target/release/edgeplane"

edgeplane-install: edgeplane-build-release  ## Install edgeplane release binary to ~/.local/bin/edgeplane
	install -m 755 crates/edgeplane/target/release/edgeplane ~/.local/bin/edgeplane
	@echo "Installed edgeplane to ~/.local/bin/edgeplane"

# ── Production image ──────────────────────────────────────────────────────────

build:  ## Build prod Docker image (tag: IMAGE:TAG and IMAGE:latest)
	docker build \
	  -t $(IMAGE):$(TAG) \
	  -t $(IMAGE):latest \
	  -f crates/edgeplane-tower/Dockerfile .

push: build  ## Push prod image to ghcr.io
	docker push $(IMAGE):$(TAG)
	docker push $(IMAGE):latest
	@echo "Pushed $(IMAGE):$(TAG) — update gitops values.yaml to deploy"

# ── Database ──────────────────────────────────────────────────────────────────

migrate:  ## Run SQLx migrations (requires DATABASE_URL)
	cargo sqlx migrate run --manifest-path crates/edgeplane-tower/Cargo.toml \
	  --source crates/edgeplane-tower/migrations

# ── Lint ──────────────────────────────────────────────────────────────────────

lint:  ## Run cargo clippy on edgeplane-tower
	cargo clippy --manifest-path crates/edgeplane-tower/Cargo.toml -- -D warnings

# ── Cleanup ───────────────────────────────────────────────────────────────────

clean:  ## Remove dev Docker volumes (destroys local DB and object storage)
	$(COMPOSE_DEV) down -v
	@echo "Dev volumes removed"
