# RZN Browser Automation Makefile

.PHONY: help build build-rust build-ext clean codebasezip logs-clear logs-follow logs-show test test-basic test-google test-dom test-dom-units dev setup install reload-ext \
	test-ext-e2e-install test-ext-e2e-run test-ext-e2e phase2 phase3 phase3-openai \
	index sg-find-stream sg-guards context-snippets agent-run agent-validate scope scope-q reducers-index invariants schema-check \
	plugins-keygen plugins-build-rzn-browser-macos plugins-verify plugins-publish-rzn-browser-local plugins-publish-rzn-browser-cloud plugins-publish-rzn-browser-prod plugins-publish-rzn-browser-all bundle-macos-share \
	release release-artifacts x-export-threads

RZN_BROWSER ?= rzn-browser

# Default target
help:
	@echo "RZN Browser Automation - Development Commands"
	@echo ""
	@echo "Build Commands:"
	@echo "  make build         - Build everything (Rust + Extension)"
	@echo "  make build-rust    - Build only Rust components"
	@echo "  make build-ext     - Build only browser extension"
	@echo "  make clean         - Clean all build artifacts"
	@echo "  make codebasezip   - Create dated lean source ZIP for external code review"
	@echo ""
	@echo "Extension Commands:"
	@echo "  make reload-ext    - Reload the browser extension"
	@echo "  make dev           - Build extension in watch mode"
	@echo ""
	@echo "Logging Commands:"
	@echo "  make logs-clear    - Clear all log files"
	@echo "  make logs-follow   - Follow current log in real-time"
	@echo "  make logs-show     - Display current log"
	@echo "  make logs-view     - Advanced log viewer with filters"
	@echo "  make logs-archive  - Archive old log files"
	@echo "  make stop-logd     - Stop the logging daemon"
	@echo ""
	@echo "Testing Commands:"
	@echo "  make test          - Run all Rust tests"
	@echo "  make test-basic    - Run basic test workflow"
	@echo "  make test-google   - Run Google search test"
	@echo "  make schema-check  - Verify actions schema ↔ generated types"
	@echo "  make test-ext-e2e  - Build + run extension Playwright e2e"
	@echo "  make phase3        - Run autonomous (dummy LLM) end-to-end"
	@echo "  make phase3-openai - Run autonomous with OpenAI (uses .env)"
	@echo ""
	@echo "DOM Testing Commands:"
	@echo "  make test-dom      - Run DOM-focused Rust tests"
	@echo "  make test-dom-units - Run DOM unit tests only"
	@echo ""
	@echo "Setup Commands:"
	@echo "  make install       - Release install (CLI + native host + Chrome/Edge/Chromium extension bundles)"
	@echo "  make setup         - Dev setup (debug-first, lighter local wiring)"
	@echo "  make release VERSION=1.2.3 - Sync versions, tag v1.2.3, and push it so GitHub Actions publishes the release"
	@echo "  make release-artifacts - Build installable release archives for the current host"
	@echo "  make doctor        - Validate local wiring (sock/manifest)"
	@echo "  make run W=...     - Run a workflow via CLI factory"
	@echo "  make x-export-threads HANDLE=... [WINDOW=week|month|custom] [SINCE=YYYY-MM-DD UNTIL=YYYY-MM-DD] [MODE=live|top] [LIMIT=10] [DOWNLOAD=1]"
	@echo "  make skill-run CMD=... - Run a skill wrapper with normalized envelope"
	@echo "  make apple-ads-keyword-recs ADAM_ID=... ADGROUP_ID=... Q=... [STOREFRONT=us]"
	@echo "  make apple-ads-portal-report REPORT_TYPE=... START=... END=... [ORG_ID=...] [CAMPAIGN_ID=...]"
	@echo "  make appstore-snapshot TERM=... [COUNTRY=us]"
	@echo ""
	@echo "Plugin Bundle Commands (rznapp install-from-file loop):"
	@echo "  make plugins-keygen - Generate .secrets/plugin-signing keys"
	@echo "  make plugins-build-rzn-browser-macos - Build signed rzn-browser ZIP (macos_universal)"
	@echo "  make plugins-verify ZIP=... PUB=... - Verify bundle ZIP (signature + sha256)"
	@echo "  make plugins-publish-rzn-browser-local - Build/upload/register/publish to local backend (http://localhost:8082)"
	@echo "  make plugins-publish-rzn-browser-cloud - Build/upload/register/publish to cloud backend (https://cloud.rzn.ai)"
	@echo "  make plugins-publish-rzn-browser-prod - Cloud backend publish path"
	@echo "  make plugins-publish-rzn-browser-all - Build once, then notify/publish both local and cloud backends"
	@echo "  make bundle-macos-share - Build friend-share ZIP (includes extension + install scripts)"
	@echo ""
	@echo "Scoped Context Commands:"
	@echo "  make scope         - Build docs/index map+context+indexes"
	@echo "  make scope-q Q=... - Quick query across scoped files"
	@echo "  make sg-guards     - Guardrails (schema DDL outside migrations)"
	@echo "  make agent-run     - Prepare an agent run (shortlist)"
	@echo "  make agent-validate - Validate changes vs shortlist + guards"

# Build everything
build: ensure-logd build-rust build-ext
	@echo "✅ Build complete!"

# Ensure rzn_logd is running
ensure-logd:
	@if ! pgrep -x rzn_logd > /dev/null; then \
		echo "🚀 Starting rzn_logd..."; \
		./target/release/rzn_logd > /dev/null 2>&1 & \
		sleep 1; \
		echo "✅ rzn_logd started"; \
	else \
		echo "✅ rzn_logd already running"; \
	fi

# Build Rust components
build-rust:
	@echo "🦀 Building Rust components..."
	RZN_LOG_ENABLED=1 cargo build --release -p rzn-browser -p rzn-native-host

# Build browser extension
build-ext:
	@echo "🌐 Building browser extension..."
	cd extension && bun install --frozen-lockfile && bun run build

# Install Playwright browsers for extension e2e
test-ext-e2e-install:
	@echo "🧩 Installing Playwright Chromium (and deps) for e2e..."
	cd extension && bun x playwright install --with-deps chromium

# Run Playwright e2e against built extension
test-ext-e2e-run:
	@echo "🧪 Running extension e2e (Playwright)..."
	cd extension && bun x playwright test --project=chromium-extension --reporter=line

# Convenience target: build extension and run e2e
test-ext-e2e: build-ext test-ext-e2e-install test-ext-e2e-run
	@echo "✅ Extension e2e complete"

# Verify generated action types match schema
schema-check:
	@node scripts/check-actions-schema.js

# Run e2e using Chrome channel (uses separate temp profile, not your main profile)
test-ext-e2e-chrome: build-ext
	@echo "🧪 Running extension e2e with Chrome channel..."
	cd extension && RZN_PW_CHANNEL=chrome bun x playwright test --project=chromium-extension --reporter=line

# Phase 2 done: enhanced actions e2e
phase2: test-ext-e2e
	@echo "🎉 Phase 2 validation complete"

# Generate dev signing keys used for local install-from-file
plugins-keygen:
	@echo "🔑 Generating plugin signing keypair..."
	cargo run -p rzn_plugin_devkit -- keygen --out .secrets/plugin-signing

# Build a signed rzn-browser bundle for rznapp "Install from file..."
# Notes:
# - This repo builds the supervisor CLI (`rzn-browser`) and native messaging host
#   (`rzn-native-host`) so the browser toolchain is developed and released together.
plugins-build-rzn-browser-macos:
	@echo "📦 Building rzn-browser plugin ZIP (macos_universal)..."
	@KEY_PATH="$${RZN_PLUGIN_SIGNING_KEY:-.secrets/plugin-signing/ed25519.private}"; \
	if [ ! -f "$$KEY_PATH" ] && [ -f "../rznapp/.secrets/plugin-signing/ed25519.private" ]; then \
		KEY_PATH="../rznapp/.secrets/plugin-signing/ed25519.private"; \
	fi; \
	if [ ! -f "$$KEY_PATH" ]; then \
		echo "[ERROR] Missing signing key at $$KEY_PATH"; \
		echo "       Run: make plugins-keygen"; \
		echo "       Or set: RZN_PLUGIN_SIGNING_KEY=/path/to/ed25519.private"; \
		exit 1; \
	fi; \
	cargo build --release -p rzn-browser; \
	cargo build --release -p rzn-native-host; \
	RZN_BROWSER_BIN_MACOS="$(PWD)/target/release/rzn-browser" \
	RZN_NATIVE_HOST_BIN_MACOS="$(PWD)/target/release/rzn-native-host" \
	cargo run -p rzn_plugin_devkit -- build \
		--config scripts/plugins/config/rzn-browser.json \
		--platform macos_universal \
		--key "$$KEY_PATH" \
		--out dist/plugins

plugins-verify:
	@if [ -z "$(ZIP)" ] || [ -z "$(PUB)" ]; then \
		echo "Usage: make plugins-verify ZIP=dist/plugins/.../rzn-browser-...zip PUB=.secrets/plugin-signing/ed25519.public"; \
		exit 1; \
	fi
	cargo run -p rzn_plugin_devkit -- verify --zip "$(ZIP)" --public "$(PUB)"

# Build a shareable macOS bundle for friends (extension + native host + CLI + setup scripts).
bundle-macos-share:
	@echo "📦 Building macOS friend-share bundle..."
	@bash scripts/package_macos_arm64_bundle.sh

OUTPUT_DIR ?= artifacts

# Package only source/config files for external code review.
# Optional: make codebasezip OUTPUT_DIR=/tmp
codebasezip:
	@bash scripts/package-code-for-architect.sh --output-dir "$(OUTPUT_DIR)"

# Clean build artifacts
clean:
	@echo "🧹 Cleaning build artifacts..."
	cargo clean
	rm -rf extension/dist extension/dist-*
	rm -rf logs/*.log

# Development mode for extension
dev:
	@echo "👨‍💻 Starting extension development mode..."
	cd extension && bun x vite

# Clear logs
logs-clear:
	@echo "🧹 Clearing logs..."
	@rm -rf logs/*.log
	@if [ -f ~/rzn_build.log ]; then \
		mv ~/rzn_build.log ~/rzn_build.log.$(date +%Y%m%d_%H%M%S); \
		echo "✅ Archived rzn_build.log"; \
	fi
	@./scripts/logger.sh clear

# Follow logs
logs-follow:
	@if [ -f ~/rzn_build.log ]; then \
		echo "📊 Following unified log (Ctrl+C to stop)..."; \
		tail -f ~/rzn_build.log; \
	else \
		./scripts/logger.sh follow; \
	fi

# Show current log
logs-show:
	@if [ -f ~/rzn_build.log ]; then \
		cat ~/rzn_build.log; \
	else \
		./scripts/logger.sh show; \
	fi

# Archive old logs
logs-archive:
	@echo "📦 Archiving old logs..."
	@mkdir -p logs/archive
	@if [ -f ~/rzn_build.log.old ]; then \
		mv ~/rzn_build.log.old logs/archive/rzn_build_$(date +%Y%m%d_%H%M%S).log; \
		echo "✅ Archived old log"; \
	fi
	@find logs -name "*.log" -mtime +7 -exec mv {} logs/archive/ \;
	@echo "✅ Logs archived"

# Advanced log viewer
logs-view:
	@./scripts/view-logs.sh -h

# Run all tests
test:
	@echo "🧪 Running Rust tests..."
	cargo test

# Test basic workflow
test-basic: ensure-logd
	RZN_LOG_ENABLED=1 ./scripts/logger.sh run workflows/test-basic.json

# Test Google search
test-google: ensure-logd
	RZN_LOG_ENABLED=1 ./scripts/logger.sh run workflows/google.search.v1.json --param search_query="rust programming"

# Full setup (dev-first defaults; avoids heavy release builds unless you ask for them)
setup:
	@echo "📦 Running setup..."
	bash ./setup.sh

# Install release artifacts into stable locations and expose the CLI on PATH.
install:
	@echo "📦 Running install (release + global CLI wiring)..."
	bash ./install.sh

release:
	@if [ -z "$(VERSION)" ]; then \
		echo "Usage: make release VERSION=1.2.3 [RELEASE_PUSH=0] [RELEASE_SKIP_CHECKS=1]"; \
		exit 1; \
	fi
	@echo "🏷️ Preparing release v$(VERSION)..."
	bash ./scripts/release/release.sh "$(VERSION)"

# Validate local native-host wiring for primary Chromium-family browsers.
doctor:
	@$(RZN_BROWSER) native-host doctor --browser chrome
	@$(RZN_BROWSER) native-host doctor --browser edge
	@$(RZN_BROWSER) native-host doctor --browser chromium

# Run a workflow via the CLI factory path (pipe)
# Usage: make run W=workflows/google/google-search.json PARAMS='--param search_query="rust lang"'
run:
	@if [ -z "$(W)" ]; then \
		echo "Usage: make run W=path/to/workflow.json [PARAMS='...']"; \
		exit 1; \
	fi
	@cargo run -p rzn-browser -- run "$(W)" $(strip $(PARAMS))

# Run workflow skill wrapper (normalized JSON envelope)
# Usage: make skill-run CMD=amazon_search ARGS='--query "wireless mouse"'
skill-run:
	@if [ -z "$(CMD)" ]; then \
		echo "Usage: make skill-run CMD=<amazon_search|amazon_product|appstore_search|appstore_details|g2_search|g2_product|capterra_search|capterra_product|etsy_search|etsy_listing|apple_ads_keyword_recs|apple_ads_keyword_suggest|apple_ads_portal_report|appstore_search_snapshot> [ARGS='...']"; \
		exit 1; \
	fi
	@./skills/amazon-appstore-workflows/scripts/run_workflow.sh "$(CMD)" $(ARGS)

# One-command skill targets (no env setup)
# Usage: make amazon-search Q="wireless mouse"
amazon-search:
	@if [ -z "$(Q)" ]; then \
		echo "Usage: make amazon-search Q=\"<search query>\""; \
		exit 1; \
	fi
	@./skills/amazon-search/scripts/run.sh --query "$(Q)" $(ARGS)

# Usage: make amazon-product URL="https://www.amazon.com/dp/B07FZ8S74R"
amazon-product:
	@if [ -z "$(URL)" ]; then \
		echo "Usage: make amazon-product URL=\"<amazon product url>\""; \
		exit 1; \
	fi
	@./skills/amazon-product/scripts/run.sh --product-url "$(URL)" $(ARGS)

# Usage: make appstore-search Q="notion"
appstore-search:
	@if [ -z "$(Q)" ]; then \
		echo "Usage: make appstore-search Q=\"<app query>\""; \
		exit 1; \
	fi
	@./skills/appstore-search/scripts/run.sh --query "$(Q)" $(ARGS)

# Usage: make appstore-details ID="1232780281"
appstore-details:
	@if [ -z "$(ID)" ]; then \
		echo "Usage: make appstore-details ID=\"<app id>\""; \
		exit 1; \
	fi
	@./skills/appstore-details/scripts/run.sh --app-id "$(ID)" $(ARGS)

# Usage: make g2-search Q="project management"
g2-search:
	@if [ -z "$(Q)" ]; then \
		echo "Usage: make g2-search Q=\"<search query>\""; \
		exit 1; \
	fi
	@./skills/g2-search/scripts/run.sh --query "$(Q)" $(ARGS)

# Usage: make g2-product URL="https://www.g2.com/products/notion/reviews"
g2-product:
	@if [ -z "$(URL)" ]; then \
		echo "Usage: make g2-product URL=\"<g2 product url>\""; \
		exit 1; \
	fi
	@./skills/g2-product-details-reviews/scripts/run.sh --product-url "$(URL)" $(ARGS)

# Usage: make capterra-search Q="crm"
capterra-search:
	@if [ -z "$(Q)" ]; then \
		echo "Usage: make capterra-search Q=\"<search query>\""; \
		exit 1; \
	fi
	@./skills/capterra-search/scripts/run.sh --query "$(Q)" $(ARGS)

# Usage: make capterra-product URL="https://www.capterra.com/p/.../"
capterra-product:
	@if [ -z "$(URL)" ]; then \
		echo "Usage: make capterra-product URL=\"<capterra product url>\""; \
		exit 1; \
	fi
	@./skills/capterra-product-details-reviews/scripts/run.sh --product-url "$(URL)" $(ARGS)

# Usage: make etsy-search Q="leather wallet"
etsy-search:
	@if [ -z "$(Q)" ]; then \
		echo "Usage: make etsy-search Q=\"<search query>\""; \
		exit 1; \
	fi
	@./skills/etsy-search/scripts/run.sh --query "$(Q)" $(ARGS)

# Usage: make etsy-listing URL="https://www.etsy.com/listing/..."
etsy-listing:
	@if [ -z "$(URL)" ]; then \
		echo "Usage: make etsy-listing URL=\"<etsy listing url>\""; \
		exit 1; \
	fi
	@./skills/etsy-listing-details-reviews/scripts/run.sh --listing-url "$(URL)" $(ARGS)

# Usage: make apple-ads-keyword-recs ADAM_ID="123" ADGROUP_ID="456" Q="budget app" STOREFRONT="us"
apple-ads-keyword-recs:
	@if [ -z "$(ADAM_ID)" ] || [ -z "$(ADGROUP_ID)" ] || [ -z "$(Q)" ]; then \
		echo "Usage: make apple-ads-keyword-recs ADAM_ID=\"<adam id>\" ADGROUP_ID=\"<adgroup id>\" Q=\"<keyword seed>\" [STOREFRONT=\"us\"]"; \
		exit 1; \
	fi
	@./skills/apple-ads-keyword-recs/scripts/run.sh --adam-id "$(ADAM_ID)" --adgroup-id "$(ADGROUP_ID)" --query "$(Q)" $(if $(STOREFRONT),--storefront "$(STOREFRONT)",) $(ARGS)

# Usage: make apple-ads-portal-report REPORT_TYPE="campaigns" START="2026-02-01" END="2026-02-15" ORG_ID="123"
apple-ads-portal-report:
	@if [ -z "$(REPORT_TYPE)" ] || [ -z "$(START)" ] || [ -z "$(END)" ]; then \
		echo "Usage: make apple-ads-portal-report REPORT_TYPE=\"<report type>\" START=\"<YYYY-MM-DD>\" END=\"<YYYY-MM-DD>\" [ORG_ID=\"<org id>\"] [CAMPAIGN_ID=\"<campaign id>\"]"; \
		exit 1; \
	fi
	@./skills/apple-ads-portal-report/scripts/run.sh --report-type "$(REPORT_TYPE)" --start-date "$(START)" --end-date "$(END)" $(if $(ORG_ID),--organization-id "$(ORG_ID)",) $(if $(CAMPAIGN_ID),--campaign-id "$(CAMPAIGN_ID)",) $(ARGS)

# Usage: make appstore-snapshot TERM="budget app" COUNTRY="us"
appstore-snapshot:
	@if [ -z "$(TERM)" ]; then \
		echo "Usage: make appstore-snapshot TERM=\"<search term>\" [COUNTRY=\"us\"]"; \
		exit 1; \
	fi
	@./skills/appstore-search-snapshot/scripts/run.sh --term "$(TERM)" $(if $(COUNTRY),--country "$(COUNTRY)",) $(ARGS)

# Reload extension (requires extension ID)
reload-ext:
	@echo "🔄 Reloading extension..."
	@echo "Note: This requires manual reload in chrome://extensions/"
	@echo "1. Open chrome://extensions/"
	@echo "2. Find 'RZN Browser Automation'"
	@echo "3. Click the refresh button"
	@open -a "Google Chrome" "chrome://extensions/"

# Quick workflow to build and test
quick-test: build-ext test-basic

# Development workflow: clear logs, build, test
dev-test: logs-clear build-ext test-basic logs-follow

# Stop logging daemon
stop-logd:
	@echo "🛑 Stopping rzn_logd..."
	@pkill -x rzn_logd || true
	@echo "✅ rzn_logd stopped"

# Test logging integration
test-logging:
	@./scripts/test-logging-integration.sh

# ============ Phase 3 (LLM Autonomous) ============

# Run autonomous planner with dummy provider (no API keys)
phase3:
	@echo "🤖 Running Phase 3 (LLM autonomous) with dummy LLM provider..."
	@echo "   Set LLM_PROVIDER=dummy to bypass API keys"
	LLM_PROVIDER=dummy cargo build --release -p rzn-browser -p rzn-native-host
	LLM_PROVIDER=dummy ./target/release/rzn-browser llm-auto "Search Google for OpenAI and extract the first 3 results" --max-steps 10 || true

# Run autonomous planner with OpenAI (reads .env / environment)
phase3-openai:
	@echo "🤖 Running Phase 3 (LLM autonomous) with OpenAI provider..."
	@echo "   Ensure:"
	@echo "   - Extension is loaded (dist/chrome) and connected to the native host"
	@echo "   - .env exports: LLM_PROVIDER=openai, OPENAI_API_KEY=..., OPENAI_MODEL_PLANNING=..., OPENAI_MAX_TOKENS=..., RZN_ALLOWED_HOSTS=..."
	@echo "   - Start with small step cap (e.g., --max-steps 8)"
	cargo build --release -p rzn-browser -p rzn-native-host
	./target/release/rzn-browser llm-auto "Search Google for OpenAI and extract the first 3 results" --max-steps 8 || true

# ============ DOM Testing Targets ============

# Run the maintained DOM-focused Rust tests
test-dom: test-dom-units
	@echo "✅ DOM test suite complete!"

# Run DOM unit tests only
test-dom-units:
	@echo "🦀 Running DOM unit tests..."
	@cargo test --package rzn_plan --test dom_integration_test -- --test-threads=1

# DOM validation workflow
dom-validate: build-rust test-dom-units
	@echo "✅ DOM validation complete!"

# ============ Scoped Context: Index / Guards / Agent Flow ============

SHELL := /bin/bash

index:
	@mkdir -p docs/index
	@./scripts/gen-tree.sh > docs/index/TREE.md
	@DEPTH=$${DEPTH:-3} ./scripts/gen-tree.sh $$DEPTH > docs/index/TREE_DEPTH_$$DEPTH.md
	@rg -n --hidden -S \
		-e "chrome\\.runtime\\.onMessage|chrome\\.runtime\\.sendMessage|chrome\\.tabs\\.sendMessage|window\\.postMessage|__rznExecuteStep|captureEnhancedDOMSnapshot|dispatch\\(|createSlice\\(|builder\\.addCase|serde_json|tokio|mpsc|websocket|postMessage" \
		--glob '!target/**' --glob '!extension/dist-*/**' --glob '!extension/dist/**' --glob '!node_modules/**' \
		| tee docs/index/HOTSPOTS.rg >/dev/null
	@echo "✅ Wrote docs/index/TREE.md and HOTSPOTS.rg"

sg-find-stream:
	@if command -v sg >/dev/null 2>&1; then \
		sg -p scripts/ast-grep/streaming/find_ts_runtime_listeners.yaml || true; \
		sg -p scripts/ast-grep/streaming/find_rust_runtime_emits.yaml || true; \
	else \
		rg -n "chrome\\.runtime\\.onMessage|window\\.addEventListener\\([^)]*message|port\\.onMessage|browser\\.runtime\\.onMessage" extension/src || true; \
		rg -n "send_message|emit|post_message|mpsc|tokio|serde_json" crates || true; \
	fi

sg-guards:
	@STRICT=$${STRICT:-0} bash -lc './scripts/sg-guards.sh'

context-snippets:
	@./scripts/gen-context.sh

reducers-index:
	@./scripts/gen-reducers-index.sh

invariants:
	@./scripts/gen-invariants.sh

agent-run:
	@./scripts/agent/agent-run.sh

agent-validate:
	@./scripts/agent/agent-validate.sh

scope: index context-snippets reducers-index invariants
	@./scripts/scope.sh

scope-q:
	@./scripts/scope-q.sh "$$Q"
