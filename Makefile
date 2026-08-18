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
WWW    := apps/www

# Overridable: make server VAULT_ROOT=~/my-vaults
#
# VAULT_ROOT is a directory *containing* one directory per vault, not a vault
# itself. Pointing it at a vault would make the server treat that vault's own
# folders as vaults.
VAULT_ROOT ?= .dev/vaults
STATE ?= .dev/state
TOKEN ?= testtoken
PORT  ?= 8484
# The device-tier client test needs its own server: it consumes the bootstrap
# pairing nonce and creates the first user, both of which are once-per-server
# and both of which `auth_e2e.py` has already used up on $(PORT).
AUTH_PORT ?= 8485

.DEFAULT_GOAL := help

## help: list available targets
help:
	@echo "Storm — make <target>"
	@echo
	@sed -n 's/^## //p' $(MAKEFILE_LIST) | awk -F': ' '{printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'
	@echo
	@echo "Variables: VAULT_ROOT=$(VAULT_ROOT) STATE=$(STATE) TOKEN=$(TOKEN) PORT=$(PORT)"

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
	rm -rf "$$ROOT/.dev/live-vaults" "$$ROOT/.dev/live-state"; \
	mkdir -p "$$ROOT/.dev/live-vaults/primary"; \
	printf '# Seed\n\nA starter note.\n' > "$$ROOT/.dev/live-vaults/primary/Seed.md"; \
	"$$ROOT/$(SERVER)/target/debug/storm-server" serve \
		--vault-root "$$ROOT/.dev/live-vaults" --state "$$ROOT/.dev/live-state" \
		--token $(TOKEN) --port $(PORT) --mcp > "$$ROOT/.dev/live-server.log" 2>&1 & \
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
	VAULT_ROOT="$$ROOT/.dev/live-vaults" python3 "$$ROOT/$(SERVER)/tests/e2e.py"; \
	echo "--- mcp e2e ---"; \
	VAULT_ROOT="$$ROOT/.dev/live-vaults" python3 "$$ROOT/$(SERVER)/tests/mcp_e2e.py"; \
	echo "--- auth e2e ---"; \
	STORM_SERVER_LOG="$$ROOT/.dev/live-server.log" \
		python3 "$$ROOT/$(SERVER)/tests/auth_e2e.py"; \
	echo "--- client device-tier auth (its own fresh server) ---"; \
	rm -rf "$$ROOT/.dev/auth-vaults" "$$ROOT/.dev/auth-state"; \
	mkdir -p "$$ROOT/.dev/auth-vaults/primary"; \
	printf '# Seed\n\nA starter note.\n' > "$$ROOT/.dev/auth-vaults/primary/Seed.md"; \
	"$$ROOT/$(SERVER)/target/debug/storm-server" serve \
		--vault-root "$$ROOT/.dev/auth-vaults" --state "$$ROOT/.dev/auth-state" \
		--token $(TOKEN) --port $(AUTH_PORT) > "$$ROOT/.dev/auth-server.log" 2>&1 & \
	AUTH_PID=$$!; \
	trap 'kill $$SERVER_PID $$AUTH_PID 2>/dev/null; true' EXIT; \
	for i in $$(seq 1 60); do \
		curl -sf -o /dev/null http://127.0.0.1:$(AUTH_PORT)/v1/health && break; \
		if [ $$i -eq 60 ]; then \
			echo "auth server did not start within 30s; log follows:" >&2; \
			cat "$$ROOT/.dev/auth-server.log" >&2; \
			exit 1; \
		fi; \
		sleep 0.5; \
	done; \
	echo "--- client integration ---"; \
	cd "$$ROOT/$(CLIENT)" && flutter test test_live/ \
		--dart-define=STORM_AUTH_BASE=http://127.0.0.1:$(AUTH_PORT) \
		--dart-define=STORM_AUTH_LOG="$$ROOT/.dev/auth-server.log"

## fmt: format both codebases
fmt:
	cd $(SERVER) && cargo fmt
	cd $(CLIENT) && dart format lib test test_live

# ---- running --------------------------------------------------------

## dry-run: report what importing every vault under VAULT_ROOT would change
dry-run:
	@mkdir -p $(VAULT_ROOT)
	cd $(SERVER) && cargo run -- dry-run \
		--vault-root $(abspath $(VAULT_ROOT)) --state $(abspath $(STATE))

## server: run the sync server against VAULT_ROOT
server:
	@mkdir -p $(VAULT_ROOT)
	cd $(SERVER) && cargo run -- serve \
		--vault-root $(abspath $(VAULT_ROOT)) --state $(abspath $(STATE)) \
		--token $(TOKEN) --port $(PORT)

## client: run the Flutter app on this machine
client:
	cd $(CLIENT) && flutter run -d macos

# Overridable for a per-user install: make install-mac MAC_APPS=$HOME/Applications
MAC_APPS ?= /Applications

## install-mac: build the release app and install it into /Applications
#
# `flutter run -d macos` above is the dev loop and dies with the terminal; this
# is the copy you keep. Ad-hoc signed (the project sets CODE_SIGN_IDENTITY = -),
# which is all a locally built, locally run app needs — nothing downloads it, so
# Gatekeeper never sees a quarantine flag.
#
# The build's output is not filtered down to a success pattern: `flutter build`
# prints nothing matching on failure, and silence reads exactly like success.
# Make stops on its exit status, and `test -d` catches the rest.
install-mac:
	cd $(CLIENT) && flutter build macos --release
	@set -e; \
	APP="$(CLIENT)/build/macos/Build/Products/Release/Storm.app"; \
	test -d "$$APP" || { echo "no bundle at $$APP" >&2; exit 1; }; \
	mkdir -p "$(MAC_APPS)" 2>/dev/null || true; \
	test -w "$(MAC_APPS)" || { \
		echo "$(MAC_APPS) is not writable." >&2; \
		echo "Retry with: make install-mac MAC_APPS=\$$HOME/Applications" >&2; \
		exit 1; \
	}; \
	rm -rf "$(MAC_APPS)/Storm.app"; \
	cp -R "$$APP" "$(MAC_APPS)/Storm.app"; \
	echo "installed $(MAC_APPS)/Storm.app"

## web: build the web bundle
web:
	cd $(CLIENT) && flutter build web --release

## serve-web: build the web client and serve it from the server binary
serve-web: web
	@mkdir -p $(VAULT_ROOT)
	cd $(SERVER) && cargo run --release -- serve \
		--vault-root $(abspath $(VAULT_ROOT)) --state $(abspath $(STATE)) \
		--token $(TOKEN) --port $(PORT) \
		--web $(abspath $(CLIENT))/build/web

# ---- deployment -----------------------------------------------------

# Override for your own host: make deploy HOST=storm@192.168.1.20
HOST       ?= proxmox-mcp-vm
REMOTE_DIR ?= /srv/storm

## build-server: cross-compile a static Linux binary
build-server:
	@command -v cargo-zigbuild >/dev/null || { \
		echo "cargo-zigbuild not found. Install with:" >&2; \
		echo "  brew install zig && cargo install cargo-zigbuild" >&2; \
		echo "  rustup target add x86_64-unknown-linux-musl" >&2; \
		exit 1; \
	}
	cd $(SERVER) && cargo zigbuild --release --target x86_64-unknown-linux-musl
	@ls -la $(SERVER)/target/x86_64-unknown-linux-musl/release/storm-server | \
		awk '{printf "  %.1f MB static binary\n", $$5/1048576}'

## deploy: build and push the server and web client to HOST
deploy: build-server web
	@set -e; \
	BIN="$(SERVER)/target/x86_64-unknown-linux-musl/release/storm-server"; \
	echo "--- staging to $(HOST) ---"; \
	scp -q "$$BIN" "$(HOST):/tmp/storm-server"; \
	rsync -az --delete "$(CLIENT)/build/web/" "$(HOST):/tmp/storm-web/"; \
	rsync -az deploy/storm-backup.sh "$(HOST):/tmp/storm-backup.sh"; \
	echo "--- installing ---"; \
	ssh "$(HOST)" "set -e; \
		sudo install -m755 /tmp/storm-server /usr/bin/storm-server; \
		sudo install -m755 /tmp/storm-backup.sh /usr/bin/storm-backup.sh; \
		sudo mkdir -p $(REMOTE_DIR)/web; \
		sudo rsync -a --delete /tmp/storm-web/ $(REMOTE_DIR)/web/; \
		sudo chown -R storm:storm $(REMOTE_DIR); \
		rm -rf /tmp/storm-server /tmp/storm-web /tmp/storm-backup.sh; \
		sudo systemctl restart storm-server; \
		sleep 1; \
		systemctl is-active --quiet storm-server && echo '  storm-server active' \
			|| { echo '  FAILED — journalctl -u storm-server' >&2; exit 1; }"
	@echo "--- deployed ---"

## deploy-web: push only the web client to HOST
#
# The full `deploy` cross-compiles the server, which needs cargo-zigbuild and
# several minutes, and a presentation-layer change touches neither the binary
# nor the wire format. ServeDir reads from disk, so nothing is restarted.
#
# WEB_DIR is what the running server was actually given as `--web`, which on
# the VM is under the service account's home rather than the /srv/storm that
# deploy/README.md describes. Check it against `pgrep -af storm-server` before
# assuming, and no sudo: the directory belongs to the user we ssh in as.
WEB_DIR ?= /home/dewansh/storm/web

deploy-web: web
	@set -e; \
	echo "--- staging web to $(HOST):$(WEB_DIR) ---"; \
	rsync -az --delete "$(CLIENT)/build/web/" "$(HOST):$(WEB_DIR)/"
	@echo "--- deployed, now verify ---"
	@$(MAKE) --no-print-directory deploy-web-check

## deploy-web-check: do the served bytes match the local build?
#
# Flutter registers a service worker, so a browser can keep serving the client
# it cached before the push — and a stale page is indistinguishable from a
# failed deploy. Comparing hashes is the only answer that is evidence. This is
# what M11 learned: a hash-verified install proves the bytes arrived.
deploy-web-check:
	@set -e; \
	LOCAL="$(CLIENT)/build/web/main.dart.js"; \
	test -f "$$LOCAL" || { echo "no local build — run: make web" >&2; exit 1; }; \
	WANT=$$(shasum -a 256 "$$LOCAL" | cut -d' ' -f1); \
	GOT=$$(ssh "$(HOST)" "curl -sf http://127.0.0.1:$(PORT)/main.dart.js | \
		sha256sum | cut -d' ' -f1"); \
	echo "  local:  $$WANT"; \
	echo "  served: $$GOT"; \
	if [ "$$WANT" = "$$GOT" ]; then \
		echo "  the server is serving this build"; \
	else \
		echo "  MISMATCH — the server is serving something else" >&2; \
		exit 1; \
	fi
	@echo "  hard-reload the browser: the service worker caches the old client"

## deploy-check: is the deployed server healthy?
deploy-check:
	@ssh "$(HOST)" "curl -sf -o /dev/null -w '  health: HTTP %{http_code}\n' \
		http://127.0.0.1:$(PORT)/v1/health || echo '  server not responding'"
	@ssh "$(HOST)" "systemctl is-active storm-server | sed 's/^/  systemd: /'"
	@ssh "$(HOST)" "systemctl list-timers storm-backup.timer --no-pager | \
		sed -n '2p' | sed 's/^/  backup: /'" 2>/dev/null || true

# ---- housekeeping ---------------------------------------------------

## codegen: regenerate drift's database code
codegen:
	cd $(CLIENT) && dart run build_runner build

## www: build the marketing site (apps/www → dist)
www:
	cd $(WWW) && npm ci && npm run build

## www-dev: Astro dev server for the marketing site
www-dev:
	cd $(WWW) && npm install && npm run dev

## clean: remove build output and local dev data
clean:
	cd $(SERVER) && cargo clean
	cd $(CLIENT) && flutter clean
	rm -rf $(WWW)/dist $(WWW)/.astro
	rm -rf .dev

.PHONY: help check lint test test-server test-client test-live fmt \
        dry-run server client web serve-web www www-dev codegen clean \
        deploy-web deploy-web-check \
        build-server deploy deploy-check
