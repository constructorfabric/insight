# ============================================================================
# Insight — shift-left developer loop.
#
# PRINCIPLE: do as much as you can LOCALLY, then CI re-runs the SAME commands
# as the mandatory gate. This Makefile is the single source of those commands —
# local and CI never drift because they both call these targets.
#
#   make check      ← run before every push: everything PR CI will enforce
#   make ci-pr      ← exactly the required PR status checks, nothing more
# ============================================================================
SHELL := /usr/bin/env bash
BACKEND := src/backend
.PHONY: check ci-pr fmt lint unit coverage coverage-unit coverage-e2e coverage-gaps dbt-validate docs-map docs-check docs-validate \
        helm-check security e2e fuzz aio contracts dev dev-down help

check: fmt lint unit dbt-validate docs-check helm-check security e2e ## full pre-push gate
	@echo "✓ ALL LOCAL GATES PASSED — safe to push (CI re-runs the same)"

ci-pr: fmt lint unit dbt-validate docs-check helm-check ## exactly the blocking PR checks

fmt: ## formatting (mirrors backend-checks.yml)
	cd $(BACKEND) && cargo fmt --all -- --check

lint: ## clippy (mirrors backend-checks.yml)
	cd $(BACKEND) && cargo clippy --all-targets --all-features -- -D warnings

unit: ## unit tests: Rust + .NET identity (mirrors backend-checks.yml)
	cd $(BACKEND) && cargo test --all
	@if command -v dotnet >/dev/null 2>&1; then \
	  cd $(BACKEND)/services/identity && dotnet test Insight.Identity.sln --configuration Release; \
	else \
	  echo "⚠ dotnet not available locally — CI will enforce"; \
	fi

coverage: coverage-unit ## instrumented coverage (alias: unit; e2e separate, see coverage-e2e)

coverage-unit: ## INSTRUMENTED unit-test coverage — exact lines touched (lcov + HTML)
	@command -v cargo-llvm-cov >/dev/null || { echo "⚠ install: cargo +stable install cargo-llvm-cov"; exit 1; }
	cd $(BACKEND) && cargo llvm-cov --all-features --workspace \
	  --lcov --output-path target/coverage-unit.lcov && \
	cargo llvm-cov report --html --output-dir target/coverage-unit-html && \
	echo "→ exact-line report: $(BACKEND)/target/coverage-unit-html/index.html"

coverage-e2e: ## INSTRUMENTED e2e coverage — services built with -C instrument-coverage, e2e suite drives them, separate report
	@command -v cargo-llvm-cov >/dev/null || { echo "⚠ install: cargo +stable install cargo-llvm-cov"; exit 1; }
	@cd $(BACKEND) && \
	  source <(cargo llvm-cov show-env --export-prefix) && \
	  cargo llvm-cov clean --workspace && \
	  cargo build --workspace && \
	  cd ../ingestion/tests/e2e && INSIGHT_BIN_DIR=../../../backend/target/debug ./e2e.sh test --tb=short -q && \
	  cd ../../../backend && \
	  cargo llvm-cov report --lcov --output-path target/coverage-e2e.lcov && \
	  cargo llvm-cov report --html --output-dir target/coverage-e2e-html && \
	  echo "→ exact-line report: $(BACKEND)/target/coverage-e2e-html/index.html"
	@echo "NOTE: e2e rig must run the instrumented host binaries (not pre-built docker images)"
	@echo "      for profraw collection — rig wiring is roadmap 2.15."

dbt-validate: ## dbt parse — catches broken models before CI
	@if command -v dbt >/dev/null 2>&1; then \
	  (cd src/ingestion/dbt && dbt parse --no-partial-parse); \
	else \
	  echo "⚠ dbt not installed locally — toolbox build enforces dbt parse in CI"; \
	fi

docs-map: ## regenerate docs/DOCS_MAP.md (markdown map by category)
	python3 scripts/ci/docs_map.py

docs-check: ## docs gate (lightweight, blocking): PRD+DESIGN present + DOCS_MAP.md fresh
	python3 scripts/ci/docs_map.py --check

docs-validate: ## authoritative artifact structure + code traceability (cypilot `cpt validate`)
	@command -v cpt >/dev/null 2>&1 || { echo "⚠ install: pipx install cypilot"; exit 1; }
	cpt validate

helm-check: ## chart validation (mirrors helm-validate.yml)
	helm dependency update charts/insight >/dev/null
	helm lint --strict charts/insight
	helm template insight charts/insight --namespace insight \
	  --set ingestion.reconcile.tenantId=local-check >/dev/null && echo "✓ chart renders"

security: ## local security scans (CI enforces; local = early warning)
	@if command -v trufflehog >/dev/null 2>&1; then \
	  trufflehog git file://. --since-commit HEAD~10 --results=verified --fail; \
	else echo "⚠ trufflehog not installed (brew install trufflehog)"; fi
	@if command -v semgrep >/dev/null 2>&1; then \
	  semgrep scan --config auto --error -q; \
	else echo "⚠ semgrep not installed (brew install semgrep)"; fi
	@if command -v cargo-audit >/dev/null 2>&1; then \
	  (cd $(BACKEND) && cargo audit); \
	else echo "⚠ cargo-audit not installed"; fi

e2e: ## bronze→API end-to-end suite (mirrors e2e-bronze-to-api.yml)
	cd src/ingestion/tests/e2e && ./e2e.sh build && \
	  { ./e2e.sh test --tb=short -q; rc=$$?; ./e2e.sh down; exit $$rc; }

fuzz: ## fuzz Rust parsers/inputs (cargo-fuzz; add targets under fuzz/)
	@command -v cargo-fuzz >/dev/null || { echo "⚠ install: cargo install cargo-fuzz"; exit 0; }
	@cd $(BACKEND) && if ls fuzz/fuzz_targets/*.rs >/dev/null 2>&1; then \
	  for t in $$(cargo fuzz list); do cargo fuzz run $$t -- -max_total_time=60; done; \
	else echo "⚠ no fuzz targets yet — scaffold: cd $(BACKEND) && cargo fuzz init (roadmap 3.3)"; fi

aio: ## run ALL services as one binary against compose infra (gears-rust pattern; roadmap 2.11)
	@cd $(BACKEND) && if cargo metadata --no-deps --format-version 1 2>/dev/null | grep -q '"insight-aio"'; then \
	  cargo run -p insight-aio; \
	else echo "⚠ insight-aio crate not built yet — roadmap 2.11 (extract run() into libs + aio crate)"; fi

contracts: ## API contract gate: every HTTP service must commit specs/schemas/openapi.json (roadmap 2.10)
	@rc=0; for svc in api-gateway analytics-api identity; do \
	  f="$(BACKEND)/services/$$svc/specs/schemas/openapi.json"; \
	  if [ -f "$$f" ]; then echo "  ✓ $$svc contract artifact"; \
	  else echo "  ✗ $$svc missing $$f (no contract, no pass — activates with roadmap 2.10)"; rc=1; fi; done; exit $$rc

coverage-gaps: ## AI coverage analysis: find untested risk areas + generate tests to close them
	@command -v claude >/dev/null || { echo "⚠ claude CLI not installed (npm i -g @anthropic-ai/claude-code)"; exit 0; }
	@cd $(BACKEND) && cargo llvm-cov --all-features --workspace --json --output-path /tmp/insight-cov.json --summary-only 2>/dev/null \
	  || { echo "⚠ run 'cargo install cargo-llvm-cov' first"; exit 0; }
	@claude -p "Read /tmp/insight-cov.json (cargo-llvm-cov summary for src/backend). Cross-reference the least-covered files against their co-located specs (specs/PRD.md, specs/DESIGN.md per service) and rank the top 10 coverage gaps by RISK (auth, tenant scoping, data correctness, error paths first - not raw percent). For each gap: file:lines, which PRD/DESIGN requirement is untested, and a concrete Rust test skeleton to close it. Write the report to /tmp/insight-coverage-gaps.md and print the top 3." --allowedTools "Read,Grep,Glob" && echo "→ full report: /tmp/insight-coverage-gaps.md"

dev: ## fast local install (Kind) — the shift-left environment
	./dev-up.sh

dev-down: ## stop local stack, keep data
	./dev-down.sh

help:
	@grep -E '^[a-z0-9-]+:.*##' Makefile | awk -F':.*## ' '{printf "  %-14s %s\n", $$1, $$2}'
