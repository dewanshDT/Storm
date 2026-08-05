# Storm — cross-toolchain task runner.
#
# Two languages and two test tiers make the useful commands awkward to
# remember, and the live suites need a server started and torn down around
# them. Encoding that here keeps it out of tribal knowledge.
#
# Written for GNU Make 3.81, which is what macOS ships (from 2006). That means
# no `.ONESHELL` and no `.SHELLFLAGS` — both landed in 3.82. Every multi-step
# recipe therefore uses explicit `\` continuations so it runs in one shell.

SERVER := apps/server
CLIENT := apps/client

# Overridable: make server VAULT=~/my-vault
VAULT ?= .dev/vault
STATE ?= .dev/state
TOKEN ?= testtoken
PORT  ?= 8484

.DEFAULT_GOAL := help

## help: list available targets
help:
	@echo "Storm — make <target>"
	@echo
	@sed -n 's/^## //p' $(MAKEFILE_LIST) | awk -F': ' '{printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'
	@echo
	@echo "Variables: VAULT=$(VAULT) STATE=$(STATE) TOKEN=$(TOKEN) PORT=$(PORT)"

# ---- checks ---------------------------------------------------------

## check: lint and run both unit suites
check: lint test

## lint: clippy + dart analyze, both must be clean
lint:
	cd $(SERVER) && cargo clippy --all-targets -- -D warnings
	cd $(CLIENT) && flutter analyze

## test: both unit suites (no server needed)
test: test-server test-client

## test-server: Rust unit tests
test-server:
	cd $(SERVER) && cargo test

## test-client: Dart unit tests
test-client:
	cd $(CLIENT) && flutter test

## test-live: integration suites against a real server, started and torn down
test-live:
	@cd $(SERVER) && cargo build --quiet
	@set -e; \
	ROOT="$$PWD"; \
	if curl -sf -o /dev/null http://127.0.0.1:$(PORT)/v1/health 2>/dev/null; then \
		echo "Something is already serving on port $(PORT)." >&2; \
		echo "The tests would run against it and its vault, not ours." >&2; \
		echo "Stop it first (pkill -f storm-server) or set PORT=..." >&2; \
		exit 1; \
	fi; \
	rm -rf "$$ROOT/.dev/live-vault" "$$ROOT/.dev/live-state"; \
	mkdir -p "$$ROOT/.dev/live-vault"; \
	printf '# Seed\n\nA starter note.\n' > "$$ROOT/.dev/live-vault/Seed.md"; \
	"$$ROOT/$(SERVER)/target/debug/storm-server" \
		--vault "$$ROOT/.dev/live-vault" --state "$$ROOT/.dev/live-state" \
		--token $(TOKEN) --port $(PORT) > "$$ROOT/.dev/live-server.log" 2>&1 & \
	SERVER_PID=$$!; \
	trap 'kill $$SERVER_PID 2>/dev/null; true' EXIT; \
	for i in $$(seq 1 60); do \
		curl -sf -o /dev/null http://127.0.0.1:$(PORT)/v1/health && break; \
		if [ $$i -eq 60 ]; then \
			echo "server did not start within 30s; log follows:" >&2; \
			cat "$$ROOT/.dev/live-server.log" >&2; \
			exit 1; \
		fi; \
		sleep 0.5; \
	done; \
	echo "--- server e2e ---"; \
	VAULT="$$ROOT/.dev/live-vault" python3 "$$ROOT/$(SERVER)/tests/e2e.py"; \
	echo "--- client integration ---"; \
	cd "$$ROOT/$(CLIENT)" && flutter test test_live/

## fmt: format both codebases
fmt:
	cd $(SERVER) && cargo fmt
	cd $(CLIENT) && dart format lib test test_live

# ---- running --------------------------------------------------------

## dry-run: report what importing VAULT would change, writing nothing
dry-run:
	cd $(SERVER) && cargo run -- \
		--vault $(abspath $(VAULT)) --state $(abspath $(STATE)) --dry-run

## server: run the sync server against VAULT
server:
	@mkdir -p $(VAULT)
	cd $(SERVER) && cargo run -- \
		--vault $(abspath $(VAULT)) --state $(abspath $(STATE)) \
		--token $(TOKEN) --port $(PORT)

## client: run the Flutter app on this machine
client:
	cd $(CLIENT) && flutter run -d macos

## web: build the web bundle
web:
	cd $(CLIENT) && flutter build web --release

## serve-web: build the web client and serve it from the server binary
serve-web: web
	@mkdir -p $(VAULT)
	cd $(SERVER) && cargo run --release -- \
		--vault $(abspath $(VAULT)) --state $(abspath $(STATE)) \
		--token $(TOKEN) --port $(PORT) \
		--web $(abspath $(CLIENT))/build/web

# ---- housekeeping ---------------------------------------------------

## codegen: regenerate drift's database code
codegen:
	cd $(CLIENT) && dart run build_runner build

## clean: remove build output and local dev data
clean:
	cd $(SERVER) && cargo clean
	cd $(CLIENT) && flutter clean
	rm -rf .dev

.PHONY: help check lint test test-server test-client test-live fmt \
        dry-run server client web serve-web codegen clean
