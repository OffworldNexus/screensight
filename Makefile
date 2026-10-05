# Screensight — developer entry points.
#
# The Rust device lives in `device/`; the Home Assistant integration lives in
# `custom_components/screensight/`. Python is managed with `uv`.
.DEFAULT_GOAL := help

CARGO ?= cargo
CROSS ?= cross
TARGET ?= aarch64-unknown-linux-gnu

.PHONY: help
help: ## Show this help
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}'

## -- Rust device ------------------------------------------------------------

.PHONY: device
device: ## Build the headless device daemon + CLI (no GPU/gpui needed)
	$(CARGO) build -p screensight

.PHONY: device-gui
device-gui: ## Build the device with the GPUI panel renderer
	$(CARGO) build -p screensight --features gui

.PHONY: check-gui
check-gui: ## Type-check the GPUI renderer without linking
	$(CARGO) check -p screensight --features gui

.PHONY: test-device
test-device: ## Run the Rust unit + integration tests
	$(CARGO) test -p screensight

.PHONY: run-headless
run-headless: ## Run screensightd headlessly (dev host; state under /tmp)
	SCREENSIGHT_STATE_DIR=/tmp/opencode/screensight-state \
	SCREENSIGHT_CONTROL_SOCKET=/tmp/opencode/screensight-state/control.sock \
	$(CARGO) run -p screensight --bin screensightd -- --headless

.PHONY: fmt-device
fmt-device: ## cargo fmt
	$(CARGO) fmt -p screensight

.PHONY: lint-device
lint-device: ## cargo fmt --check + clippy (all features)
	$(CARGO) fmt -p screensight -- --check
	$(CARGO) clippy -p screensight --all-targets --features gui -- -D warnings

.PHONY: cross
cross: ## Cross-compile the device for the Raspberry Pi
	$(CROSS) build --release --target $(TARGET) -p screensight --features gui

.PHONY: deb
deb: ## Build the aarch64 .deb package
	./deploy/deb/build.sh

.PHONY: deploy
deploy: ## Build + install the .deb on the Pi
	./deploy/deploy.sh

## -- Home Assistant integration --------------------------------------------

.PHONY: ha-setup
ha-setup: ## Install Python dependencies with uv
	uv sync

.PHONY: ha-fmt
ha-fmt: ## Format the Python integration
	uv run ruff format .

.PHONY: ha-lint
ha-lint: ## Lint + type-check the Python integration
	uv run ruff format --check .
	uv run ruff check .
	uv run mypy custom_components/screensight

.PHONY: ha-test
ha-test: ## Run the Home Assistant tests (unit + BDD)
	uv run pytest

.PHONY: ha-bdd
ha-bdd: ## Run only the BDD scenarios
	uv run pytest -m bdd

.PHONY: ha-dev
ha-dev: ## Start a throwaway local Home Assistant Core (auto-onboarded, dev/dev)
	./scripts/ha-dev.sh up

.PHONY: ha-dev-fg
ha-dev-fg: ## Run the throwaway HA in the foreground (Ctrl+C safe)
	./scripts/ha-dev.sh up-fg

.PHONY: ha-dev-logs
ha-dev-logs: ## Follow the throwaway HA logs
	./scripts/ha-dev.sh logs

.PHONY: ha-dev-restart
ha-dev-restart: ## Restart the throwaway HA
	./scripts/ha-dev.sh restart

.PHONY: ha-dev-stop
ha-dev-stop: ## Stop and remove the throwaway HA
	./scripts/ha-dev.sh down

.PHONY: ha-dev-reset
ha-dev-reset: ## Wipe the throwaway HA config and container
	./scripts/ha-dev.sh reset

.PHONY: emulate
emulate: ## Run the device panel on this desktop ("emulated Pi")
	./scripts/emulate-device.sh

.PHONY: smoke
smoke: ## Full headless end-to-end smoke test of the device pipeline
	./scripts/smoke-test.sh

## -- Everything -------------------------------------------------------------

.PHONY: lint
lint: lint-device ha-lint ## Lint Rust + Python

.PHONY: test
test: test-device ha-test ## Run all tests
