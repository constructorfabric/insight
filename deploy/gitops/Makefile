# Insight GitOps — engineer-driven deploy entry point.
#
# Reference: ../components/deployment/gitops/README.md
#
# This is a SAMPLE gitops layout intended to be copied into your own
# private repo and adapted (cluster names, hostnames, image tags, secret
# backend, CI). The umbrella chart it pulls (oci://ghcr.io/constructorfabric/
# charts/insight) is published from the public Insight repo per merge to
# main, so this directory plus an actual cluster is enough to deploy.
#
# Quick reference (L0 = cluster prereqs, L2 = shared infra, L3 = umbrella):
#   make doctor                                    # check tooling
#   make bootstrap ENV=<env>                       # L0: ingress-nginx, cert-manager, sealed-secrets, namespaces
#   make system-mariadb ENV=<env>                  # L2: shared infra (one target per service)
#   make system-clickhouse ENV=<env>               # L2
#   make system-redis ENV=<env>                    # L2
#   make system-redpanda ENV=<env>                 # L2
#   make system-redpanda-console ENV=<env>         # L2
#   make system-airbyte ENV=<env>                  # L2 (+ post-install setup wizard)
#   make system-argo ENV=<env>                     # L2 (+ supplemental RBAC)
#   make deploy ENV=<env>                          # L3: umbrella app only — alias for deploy-app
#   make deploy ENV=production CONFIRM=yes-deploy-production
#   make rollback ENV=<env>                        # helm rollback (umbrella)
#   make status ENV=<env>                          # umbrella rollout health
#   make system-status ENV=<env>                   # what's installed in insight-infra
#   make seal-secret ENV=<env> NAMESPACE=<ns> NAME=<name>
#       NAMESPACE = insight-infra for L2 secrets, insight for L3.
#       Cleartext for the Secret is fetched by `scripts/secret-fetch.sh`
#       (sample stub reads from a local YAML — point it at your own
#       password manager). Or use VALUE_FILE=… for one-off seals.
#
# This repo is settings-only. The umbrella chart is published per merge
# to oci://ghcr.io/constructorfabric/charts/insight; INSIGHT_VERSION pins one
# semver from .insight-version.

ENV              ?= local
# Single source of truth for per-env cluster topology. Drives bootstrap,
# system, seal, and deploy targets. Schema: see
# environments/local/inventory.yaml.
INVENTORY        := environments/$(ENV)/inventory.yaml

# Inventory-driven values. CLI flags and shell env vars override (via ?=).
# If the inventory file is missing, `$(shell yq ...)` returns "" and the
# downstream `inventory-present` / `kube-ctx-required` guards fail loud.
KUBE_CTX         ?= $(shell yq -r '.kubeContext // ""' $(INVENTORY) 2>/dev/null)
NS_APP           ?= $(shell yq -r '.namespaces.services // "insight"'        $(INVENTORY) 2>/dev/null)
NS_INFRA         ?= $(shell yq -r '.namespaces.infra    // "insight-infra"'  $(INVENTORY) 2>/dev/null)
RELEASE          ?= $(shell yq -r '.release             // "insight"'        $(INVENTORY) 2>/dev/null)

# Back-compat alias for older targets still referencing NAMESPACE.
NAMESPACE        ?= $(NS_APP)
CHART            ?= oci://ghcr.io/constructorfabric/charts/insight
# Local umbrella chart source (this repo's working tree) for validation
LOCAL_CHART      ?= ../../charts/insight
INSIGHT_VERSION  ?= $(shell cat .insight-version 2>/dev/null)
VALUES           ?= environments/$(ENV)/values.yaml
TIMEOUT          ?= 10m
RENDER_DIR       := .deploy

# Extra flags appended to helm invocations. Default empty.
#
# HELM_EXTRA_FLAGS:   passed to ALL helm calls (bootstrap-*, system-*,
#                     umbrella deploy, show/template/list/status/rollback).
#                     Use for flags valid on every helm command:
#                       `--plain-http` when pulling from a local HTTP OCI
#                         registry (e.g. oci://localhost:5001/...)
#                       `--debug` when a step is hanging and you want
#                         helm to print every kubectl API call it makes
#                         (verbose; great for diagnosing a stuck install).
#                     Example: HELM_EXTRA_FLAGS=--debug make deploy ENV=local
# HELM_UPGRADE_FLAGS: passed ONLY to the umbrella `helm upgrade --install`.
#                     Use for apply-time flags like `--force-conflicts`
#                     (SSA ownership override) that error out on
#                     read-only `helm show` / `helm template`.
HELM_EXTRA_FLAGS   ?=
HELM_UPGRADE_FLAGS ?=

# ── L2 system services (shared infra in NS_INFRA) ──
# Each service has chart-ref + version pinned here. Shared base values
# live at system/<svc>/values.yaml; per-env overlays at
# environments/$(ENV)/<svc>-values.yaml (added only when an env differs).
# system-* targets run them manually — no top-level chain; each cluster
# picks which services it self-hosts vs. swaps for managed endpoints.

MARIADB_RELEASE          ?= mariadb
MARIADB_CHART            ?= oci://registry-1.docker.io/bitnamicharts/mariadb
MARIADB_VERSION          ?= 25.1.1

CLICKHOUSE_RELEASE       ?= clickhouse
CLICKHOUSE_CHART         ?= oci://registry-1.docker.io/bitnamicharts/clickhouse
CLICKHOUSE_VERSION       ?= 9.4.4

REDIS_RELEASE            ?= redis
REDIS_CHART              ?= oci://registry-1.docker.io/bitnamicharts/redis
REDIS_VERSION            ?= 25.5.2

REDPANDA_RELEASE         ?= redpanda
REDPANDA_CHART           ?= redpanda/redpanda
REDPANDA_VERSION         ?= 26.1.3

REDPANDA_CONSOLE_RELEASE ?= redpanda-console
REDPANDA_CONSOLE_CHART   ?= redpanda/console
REDPANDA_CONSOLE_VERSION ?= 3.7.0

AIRBYTE_RELEASE  ?= airbyte
AIRBYTE_VERSION  ?= 1.9.2

ARGO_RELEASE     ?= argo-workflows
ARGO_VERSION     ?= 1.0.13
ARGO_RBAC_TMPL   ?= bootstrap/argo-rbac.yaml.template
ARGO_WORKFLOW_SA ?= argo-workflow

# Observability stack (LGTM — logs first). Installed only when the bundled
# stack is enabled via inventory.system.{loki,alloy,grafana}; otherwise the
# host cluster collects from stdout and these targets are skipped.
# Chart versions verified latest via `helm search repo grafana/<chart>
# --versions` (bump deliberately, same discipline as the pins above).
LOKI_RELEASE     ?= loki
LOKI_CHART       ?= grafana/loki
LOKI_VERSION     ?= 7.0.0      # app 3.6.7

ALLOY_RELEASE    ?= alloy
ALLOY_CHART      ?= grafana/alloy
ALLOY_VERSION    ?= 1.9.0      # app v1.16.3

GRAFANA_RELEASE  ?= grafana
GRAFANA_CHART    ?= grafana/grafana
GRAFANA_VERSION  ?= 10.5.15    # app 12.3.1

# Per-cluster bootstrap controllers (ingress, cert-manager, sealed-secrets).
# Versions pinned here; bump deliberately. Per-env overrides go in
# bootstrap/<env>/<controller>-values.yaml. Namespaces fall through to
# inventory.namespaces.<controller> for rare-but-allowed overrides.
INGRESS_NGINX_NAMESPACE  ?= $(shell yq -r '.namespaces.ingressNginx  // "ingress-nginx"' $(INVENTORY) 2>/dev/null)
INGRESS_NGINX_RELEASE    ?= ingress-nginx
INGRESS_NGINX_VERSION    ?= 4.15.1

CERT_MANAGER_NAMESPACE   ?= $(shell yq -r '.namespaces.certManager   // "cert-manager"' $(INVENTORY) 2>/dev/null)
CERT_MANAGER_RELEASE     ?= cert-manager
CERT_MANAGER_VERSION     ?= 1.20.2

SEALED_SECRETS_NAMESPACE ?= $(shell yq -r '.namespaces.sealedSecrets // "kube-system"' $(INVENTORY) 2>/dev/null)
SEALED_SECRETS_RELEASE   ?= sealed-secrets-controller
SEALED_SECRETS_VERSION   ?= 2.17.4

BOOTSTRAP_DIR := bootstrap/$(ENV)

# Colour helpers (no-op if not a TTY)
_TPUT := $(shell tput colors 2>/dev/null || echo 0)
ifneq ($(_TPUT),0)
  C_RED   := $(shell tput setaf 1)
  C_GRN   := $(shell tput setaf 2)
  C_YEL   := $(shell tput setaf 3)
  C_RST   := $(shell tput sgr0)
endif

.DEFAULT_GOAL := help

.PHONY: help
help:
	@echo "Targets:"
	@echo "  make doctor                       Check required tooling"
	@echo "  make bootstrap ENV=$(ENV)         L0 — inventory-driven cluster prereqs (ingress, cert-manager, sealed-secrets)"
	@echo "  make sync                         git pull --ff-only origin main"
	@echo "  make diff ENV=$(ENV)              Show poller commits + render diff"
	@echo "  make deploy ENV=$(ENV)            L3 only — helm install/upgrade the umbrella (alias: deploy-app)"
	@echo "  make apply-app-secrets ENV=$(ENV)   L3 prereq — apply sealed manifests + derive *-config Secrets from insight-db-creds"
	@echo "  make system ENV=$(ENV)            L2 — install all services with inventory.system.<svc>: true"
	@echo "  make system-mariadb ENV=$(ENV)            }"
	@echo "  make system-clickhouse ENV=$(ENV)         } L2 — one-at-a-time for one-offs / rotation."
	@echo "  make system-redis ENV=$(ENV)              } Top-level 'make system' chains all enabled in inventory."
	@echo "  make system-redpanda ENV=$(ENV)           }"
	@echo "  make system-redpanda-console ENV=$(ENV)   }"
	@echo "  make system-airbyte ENV=$(ENV)            }   (Airbyte = chart + post-install setup wizard)"
	@echo "  make system-argo ENV=$(ENV)               }   (Argo Workflows = chart + supplemental RBAC apply)"
	@echo "  make system-loki ENV=$(ENV)               }"
	@echo "  make system-alloy ENV=$(ENV)              } L2 — observability (LGTM, bundled mode)."
	@echo "  make system-grafana ENV=$(ENV)            }"
	@echo "  make system-status ENV=$(ENV)             Show what's installed in insight-infra"
	@echo "  make rollback ENV=$(ENV)          Roll back the umbrella to previous revision"
	@echo "  make status ENV=$(ENV)            kubectl rollout status + helm status"
	@echo "  make tag                          Tag current commit as deploy-…"
	@echo "  make seal ENV=$(ENV)              Seal all secrets listed in inventory.secrets"
	@echo "  make seal-secret …                Seal one Secret via scripts/secret-fetch.sh"
	@echo "  make seal-from-file VALUE_FILE=…  Seal a Secret from a local cleartext file"
	@echo "  make seal-from-template …         Seal a *-secret-template.yaml in this repo"
	@echo "  make seal-stdin …                 Seal a Secret read from stdin"
	@echo "  make clear-seal-template …        Empty all values in a template file"
	@echo "  make fetch-cert ENV=…             kubeseal --fetch-cert into pub-cert.pem"
	@echo
	@echo "Inventory (drives bootstrap / system / seal / deploy):"
	@echo "  $(INVENTORY)"
	@echo
	@echo "Resolved values (inventory + CLI/env overrides):"
	@echo "  ENV=$(ENV)  KUBE_CTX=$(KUBE_CTX)  RELEASE=$(RELEASE)"
	@echo "  NS_APP=$(NS_APP)  NS_INFRA=$(NS_INFRA)"
	@echo "  CHART=$(CHART)"
	@echo "  INSIGHT_VERSION=$(INSIGHT_VERSION)"
	@echo "  VALUES=$(VALUES)"

# ─── Pre-flight checks ─────────────────────────────────────────────────

.PHONY: doctor
doctor:
	@bash scripts/doctor.sh

.PHONY: sync
sync:
	@git fetch --quiet origin
	@git pull --ff-only origin main

.PHONY: sync-clean
sync-clean:
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "$(C_RED)ERROR$(C_RST): working tree has uncommitted changes"; \
		git status --short; exit 1; fi
	@git fetch --quiet origin
	@LOCAL=$$(git rev-parse @); REMOTE=$$(git rev-parse @{u} 2>/dev/null || echo ""); \
	if [ -z "$$REMOTE" ]; then \
		echo "$(C_YEL)WARN$(C_RST): no upstream tracked; skipping HEAD-vs-remote check"; \
	elif [ "$$LOCAL" != "$$REMOTE" ]; then \
		echo "$(C_RED)ERROR$(C_RST): HEAD is not equal to origin/$$(git rev-parse --abbrev-ref @{u} | cut -d/ -f2-)"; \
		echo "       run 'make sync' first"; exit 1; fi

.PHONY: inventory-present
inventory-present:
	@test -f $(INVENTORY) \
		|| { echo "$(C_RED)ERROR$(C_RST): inventory not found at $(INVENTORY)"; \
		     echo "       For ENV=local: run \`make deploy ENV=local\` and the first-run wizard"; \
		     echo "       will generate it interactively."; \
		     echo "       For other envs: copy environments/local/inventory.yaml.template to"; \
		     echo "       environments/<env>/inventory.yaml, adjust kubeContext / namespaces /"; \
		     echo "       bootstrap / system / secrets, then re-run."; \
		     exit 1; }

.PHONY: kube-ctx-required
kube-ctx-required: inventory-present
	@test -n "$(KUBE_CTX)" \
		|| { echo "$(C_RED)ERROR$(C_RST): KUBE_CTX is not set"; \
		     echo "       Set it on the command line (KUBE_CTX=...), or in your shell environment,"; \
		     echo "       or add 'kubeContext: <name>' to $(INVENTORY)."; \
		     exit 1; }

.PHONY: vpn-up
vpn-up: kube-ctx-required
	@kubectl --context $(KUBE_CTX) --request-timeout=5s cluster-info >/dev/null 2>&1 \
		|| { echo "$(C_RED)ERROR$(C_RST): cannot reach cluster '$(KUBE_CTX)'; is the VPN up?"; exit 1; }

.PHONY: kube-ctx
kube-ctx: kube-ctx-required
	@CUR=$$(kubectl config current-context 2>/dev/null || echo none); \
	if [ "$$CUR" != "$(KUBE_CTX)" ]; then \
		echo "$(C_RED)ERROR$(C_RST): current kube-context is '$$CUR', expected '$(KUBE_CTX)'"; \
		echo "       run: kubectl config use-context $(KUBE_CTX)"; \
		exit 1; fi

.PHONY: confirm
confirm: inventory-present
	@if [ "$$(yq -r '.protected // false' $(INVENTORY))" = "true" ]; then \
		EXPECTED="yes-deploy-$(ENV)"; \
		if [ "$(CONFIRM)" != "$$EXPECTED" ]; then \
			echo "$(C_RED)ERROR$(C_RST): deploy to '$(ENV)' requires CONFIRM=$$EXPECTED (per inventory.protected)"; \
			exit 1; fi; \
	fi

.PHONY: chart-present
chart-present:
	@test -n "$(INSIGHT_VERSION)" \
		|| { echo "$(C_RED)ERROR$(C_RST): INSIGHT_VERSION is empty; populate .insight-version"; exit 1; }
	@helm show chart $(CHART) --version $(INSIGHT_VERSION) $(HELM_EXTRA_FLAGS) >/dev/null 2>&1 \
		|| { echo "$(C_RED)ERROR$(C_RST): chart $(CHART):$(INSIGHT_VERSION) not found in registry"; \
		     echo "       check .insight-version and that the chart was published by CI"; \
		     exit 1; }

.PHONY: values-present
values-present:
	@test -f $(VALUES) \
		|| { echo "$(C_RED)ERROR$(C_RST): values file not found at $(VALUES)"; exit 1; }

# ─── Diff and deploy ───────────────────────────────────────────────────

.PHONY: diff
diff: sync-clean values-present chart-present
	@bash scripts/render-diff.sh "$(ENV)" "$(NAMESPACE)" "$(RELEASE)" \
		"$(CHART)" "$(VALUES)" "$(RENDER_DIR)" "$(INSIGHT_VERSION)"

# Top-level deploy. For ENV=local (sandbox), `deploy` chains the full
# bring-up: wizard → bootstrap → fetch-cert → seal → system → deploy-app.
# For any other env, `deploy` keeps the narrow behavior (just upgrade
# the L3 umbrella); each cluster picks which L2 services it self-hosts
# vs. swaps for managed endpoints, so chaining `system` would be wrong.
.PHONY: deploy
ifeq ($(ENV),local)
deploy: local-up
else
deploy: deploy-app
endif

# Local sandbox bring-up. Runs the wizard if inventory.yaml is missing,
# then chains the L0/L2/L3 install. Idempotent — re-running is a no-op
# apart from helm-upgrade noise. Each sub-make re-evaluates the
# inventory, so the wizard-written inventory.yaml is visible to the
# chained targets even though this make process started without it.
.PHONY: local-up
local-up: local-wizard
	@set -a; [ -f environments/local/.env.local ] && . ./environments/local/.env.local; set +a; \
	  CTX=$$(yq -r '.kubeContext' $(INVENTORY)); \
	  if [ -z "$$CTX" ] || [ "$$CTX" = "null" ]; then \
	    echo "$(C_RED)ERROR$(C_RST): inventory.yaml missing .kubeContext after wizard"; exit 1; \
	  fi; \
	  $(MAKE) --no-print-directory bootstrap      ENV=local KUBE_CTX=$$CTX && \
	  $(MAKE) --no-print-directory fetch-cert     ENV=local KUBE_CTX=$$CTX && \
	  $(MAKE) --no-print-directory seal           ENV=local KUBE_CTX=$$CTX && \
	  $(MAKE) --no-print-directory system         ENV=local KUBE_CTX=$$CTX && \
	  $(MAKE) --no-print-directory keycloak-realm ENV=local KUBE_CTX=$$CTX && \
	  $(MAKE) --no-print-directory deploy-app     ENV=local KUBE_CTX=$$CTX

# Run the wizard only when inventory.yaml is missing. The wizard itself
# enforces non-TTY rejection. If the file is present (re-run on the
# same machine), skip silently.
.PHONY: local-wizard
local-wizard:
	@if [ -f $(INVENTORY) ]; then \
	  exit 0; \
	fi; \
	WIZARD="$$(cd ../.. && pwd)/deploy/compose/insight-init.sh"; \
	if [ ! -f "$$WIZARD" ]; then \
	  echo "$(C_RED)ERROR$(C_RST): wizard not found at $$WIZARD"; exit 1; \
	fi; \
	bash "$$WIZARD" --target=k8s-local

.PHONY: keycloak-realm
# Generate the Keycloak dev realm from the seed roster (deploy/compose/keycloak/
# gen-realm.py) and apply it as the `$(RELEASE)-keycloak-realm` ConfigMap that the
# insight-keycloak subchart mounts + imports. Runs before deploy-app in local-up.
# No-op unless keycloak.deploy=true (fakeidp installs need nothing here).
keycloak-realm: kube-ctx values-present
	@if [ "$$(yq -r '.keycloak.deploy // false' $(VALUES))" != "true" ]; then \
	  echo "$(C_YEL)skip$(C_RST) keycloak-realm (keycloak.deploy != true)"; exit 0; fi
	@GEN="$$(cd ../.. && pwd)/deploy/compose/keycloak/gen-realm.py"; \
	OUT="$(RENDER_DIR)/realm-insight-$(ENV).json"; mkdir -p "$(RENDER_DIR)"; \
	DEV_EMAIL="$$(yq -r '.fakeidp.devUserEmail // "dev@company.nonpresent"' $(VALUES))"; \
	REDIRECT="$$(yq -r '.authenticator.oidc.redirectUri // "http://localhost/auth/callback"' $(VALUES))"; \
	KC_BASE="$$(yq -r '.keycloak.hostname // ""' $(VALUES))"; \
	LB_REDIRECT="$${KC_BASE%/kc}/auth/callback"; \
	echo "$(C_GRN)→$(C_RST) generating Keycloak realm (dev-lead $$DEV_EMAIL; redirects $$REDIRECT, $$LB_REDIRECT)"; \
	python3 "$$GEN" --dev-email "$$DEV_EMAIL" \
	  --authenticator-redirect "$$REDIRECT" \
	  --authenticator-redirect "$$LB_REDIRECT" \
	  --out "$$OUT"; \
	kubectl --context $(KUBE_CTX) -n $(NS_APP) create configmap $(RELEASE)-keycloak-realm \
	  --from-file=realm-insight.json="$$OUT" --dry-run=client -o yaml | \
	  kubectl --context $(KUBE_CTX) -n $(NS_APP) apply -f -; \
	echo "$(C_GRN)applied$(C_RST) configmap $(RELEASE)-keycloak-realm → $(NS_APP)"

.PHONY: deploy-app
deploy-app: deploy-insight

# Apply L3 sealed secrets to the cluster + derive the composed
# *-config Secrets that the chart references via `existingSecret`.
# Derived Secrets are NOT sealed/committed — they're regenerated from
# the already-sealed insight-db-creds on every deploy. Idempotent.
.PHONY: apply-app-secrets
apply-app-secrets: vpn-up kube-ctx
	@SEAL_DIR=environments/$(ENV)/sealed-secrets/insight; \
	if [ ! -d "$$SEAL_DIR" ] || [ -z "$$(ls -A $$SEAL_DIR/*-sealedsecret.yaml 2>/dev/null)" ]; then \
		echo "$(C_RED)ERROR$(C_RST): no L3 sealed manifests in $$SEAL_DIR/"; \
		echo "       seal at minimum insight-db-creds before running deploy-app:"; \
		echo "         make seal-secret ENV=$(ENV) NAMESPACE=$(NS_APP) NAME=insight-db-creds"; \
		exit 1; \
	fi; \
	echo "applying L3 sealed manifests under $$SEAL_DIR/"; \
	for f in $$SEAL_DIR/*-sealedsecret.yaml; do \
		kubectl --context $(KUBE_CTX) apply -f "$$f"; \
	done
	@echo "waiting for sealed-secrets-controller to materialise insight-db-creds…"; \
	for i in $$(seq 1 30); do \
		if kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret insight-db-creds >/dev/null 2>&1; then \
			break; \
		fi; \
		sleep 1; \
	done; \
	kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret insight-db-creds >/dev/null 2>&1 \
		|| { echo "$(C_RED)ERROR$(C_RST): insight-db-creds never materialised (controller down? sealed manifest invalid?)"; exit 1; }
	@ENV=$(ENV) NS_APP=$(NS_APP) RELEASE=$(RELEASE) \
		bash scripts/compose-app-secrets.sh

.PHONY: local-chart-present
local-chart-present:
	@test -f $(LOCAL_CHART)/Chart.yaml \
		|| { echo "$(C_RED)ERROR$(C_RST): local chart not found at $(LOCAL_CHART)/Chart.yaml"; exit 1; }

.PHONY: validate-insight
validate-insight: values-present local-chart-present
	@mkdir -p $(RENDER_DIR)
	@helm dependency build $(LOCAL_CHART) $(HELM_EXTRA_FLAGS)
	@helm template $(RELEASE) $(LOCAL_CHART) $(HELM_EXTRA_FLAGS) \
		-n $(NS_APP) -f $(VALUES) \
		> $(RENDER_DIR)/validate-render-$(ENV).yaml
	@echo "$(C_GRN)rendered$(C_RST) $(LOCAL_CHART) (local source) → $(RENDER_DIR)/validate-render-$(ENV).yaml"
	@kubeconform -strict -summary < $(RENDER_DIR)/validate-render-$(ENV).yaml

.PHONY: deploy-insight
deploy-insight: sync-clean vpn-up kube-ctx confirm values-present chart-present apply-app-secrets
	@mkdir -p $(RENDER_DIR)
	@helm template $(RELEASE) $(CHART) --version $(INSIGHT_VERSION) $(HELM_EXTRA_FLAGS) \
		-n $(NS_APP) -f $(VALUES) \
		> $(RENDER_DIR)/last-render-$(ENV).yaml
	@echo "$(C_GRN)rendered$(C_RST) $(CHART):$(INSIGHT_VERSION) → $(RENDER_DIR)/last-render-$(ENV).yaml"
	@LOG=$(RENDER_DIR)/deploy-$(ENV)-$$(date -u +%Y%m%d-%H%M%S).log; \
	echo "$(C_GRN)applying$(C_RST) → log at $$LOG"; \
	helm upgrade --install $(RELEASE) $(CHART) \
		--version $(INSIGHT_VERSION) $(HELM_EXTRA_FLAGS) $(HELM_UPGRADE_FLAGS) \
		--namespace $(NS_APP) --create-namespace \
		-f $(VALUES) \
		--atomic --timeout $(TIMEOUT) \
		--history-max 10 \
		2>&1 | tee $$LOG; \
	NS_INFRA=$(NS_INFRA) KUBE_CTX=$(KUBE_CTX) \
		bash scripts/push-deploy-log.sh $(ENV) $$LOG $(INSIGHT_VERSION) || true

# Helper: chain a sub-target IF the matching inventory.<section>.<key>
# is true; otherwise print a skip message. Use with $(call …) below.
#   $(1) — inventory key (e.g. bootstrap.ingressNginx)
#   $(2) — sub-target name (e.g. bootstrap-ingress-nginx)
define _run_if_enabled
@if [ "$$(yq -r '.$(1) // false' $(INVENTORY))" = "true" ]; then \
	echo "$(C_GRN)→$(C_RST) $(2)"; \
	$(MAKE) --no-print-directory $(2) ENV=$(ENV); \
else \
	echo "$(C_YEL)skip$(C_RST) $(2) (inventory.$(1) != true)"; \
fi
endef

# ─── L2 system services ────────────────────────────────────────────────
# Each target follows the same shape: helm repo (if applicable), helm
# upgrade --install with shared base + optional per-env overlay, into
# $(NS_INFRA). Idempotent. Per-service targets remain available for
# one-off / rotation work.
#
# `make system` orchestrates them per inventory.system.<svc>.

.PHONY: system
system: inventory-present vpn-up kube-ctx
	$(call _run_if_enabled,system.mariadb,system-mariadb)
	$(call _run_if_enabled,system.clickhouse,system-clickhouse)
	$(call _run_if_enabled,system.redis,system-redis)
	$(call _run_if_enabled,system.redpanda,system-redpanda)
	$(call _run_if_enabled,system.redpandaConsole,system-redpanda-console)
	$(call _run_if_enabled,system.airbyte,system-airbyte)
	$(call _run_if_enabled,system.argoWorkflows,system-argo)
	$(call _run_if_enabled,system.loki,system-loki)
	$(call _run_if_enabled,system.alloy,system-alloy)
	$(call _run_if_enabled,system.grafana,system-grafana)
	@echo "$(C_GRN)system complete$(C_RST) → $(NS_INFRA)"

define _helm_repo_add
	helm repo add $(1) $(2) >/dev/null 2>&1 || true; helm repo update $(1) >/dev/null
endef

# Helper: render `-f system/<svc>/values.yaml [-f environments/<env>/<svc>-values.yaml]`
# Usage: $(call _system_values_args,<svc>)
_system_values_args = -f system/$(1)/values.yaml $(shell test -s environments/$(ENV)/$(1)-values.yaml && echo -f environments/$(ENV)/$(1)-values.yaml)

# Helper: require a sealed-secret named `<svc>-creds` to be present in
# the repo for this env. Apply it to $(NS_INFRA). If missing, print
# clear instructions and abort. Used by system-* targets whose chart
# has `existingSecret: <svc>-creds` in the baseline values.
define _require_system_creds
@SVC=$(1); \
SEALED="environments/$(ENV)/sealed-secrets/$(NS_INFRA)/$$SVC-creds-sealedsecret.yaml"; \
if [ -f "$$SEALED" ]; then \
  kubectl --context $(KUBE_CTX) apply -f "$$SEALED" >/dev/null; \
  echo "$(C_GRN)applied$(C_RST) $$SEALED"; \
else \
  echo "$(C_RED)ERROR$(C_RST): missing $$SEALED"; \
  echo "       The $$SVC chart expects a Secret named '$$SVC-creds' in $(NS_INFRA)."; \
  echo "       To create it:"; \
  echo "         1. Stage the cleartext Secret manifest where scripts/secret-fetch.sh"; \
  echo "            can find it (sample stub: add an entry keyed"; \
  echo "              insight-$(ENV)-$$SVC-creds"; \
  echo "            in secrets-store.yaml). See system/$$SVC/SECRETS.md for the"; \
  echo "            required keys."; \
  echo "         2. Seal it into the repo:"; \
  echo "              make seal-secret ENV=$(ENV) NAMESPACE=$(NS_INFRA) NAME=$$SVC-creds"; \
  echo "         3. Commit the resulting $$SEALED and re-run."; \
  exit 1; \
fi
endef

# Helper: opportunistically apply ANY sealed-secret manifest under
# $(NS_INFRA) whose filename starts with the service prefix. Silent
# if none exist. Used by services whose baseline does not require a
# secret (redpanda, redpanda-console, airbyte, argo-workflows), but
# whose per-env overlay may add TLS / auth / object-store creds.
define _apply_optional_system_secrets
@SVC=$(1); \
shopt -s nullglob 2>/dev/null || true; \
for f in environments/$(ENV)/sealed-secrets/$(NS_INFRA)/$$SVC*-sealedsecret.yaml; do \
  [ -e "$$f" ] || continue; \
  kubectl --context $(KUBE_CTX) apply -f "$$f" >/dev/null; \
  echo "$(C_GRN)applied$(C_RST) $$f"; \
done
endef

.PHONY: system-mariadb
system-mariadb: vpn-up kube-ctx
	$(call _require_system_creds,mariadb)
	helm upgrade --install $(MARIADB_RELEASE) $(MARIADB_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(MARIADB_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,mariadb) \
		--wait --timeout 10m

.PHONY: system-clickhouse
system-clickhouse: vpn-up kube-ctx
	$(call _require_system_creds,clickhouse)
	helm upgrade --install $(CLICKHOUSE_RELEASE) $(CLICKHOUSE_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(CLICKHOUSE_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,clickhouse) \
		--wait --timeout 10m

.PHONY: system-redis
system-redis: vpn-up kube-ctx
	$(call _require_system_creds,redis)
	helm upgrade --install $(REDIS_RELEASE) $(REDIS_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(REDIS_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,redis) \
		--wait --timeout 5m

.PHONY: system-redpanda
system-redpanda: vpn-up kube-ctx
	@$(call _helm_repo_add,redpanda,https://charts.redpanda.com)
	$(call _apply_optional_system_secrets,redpanda)
	helm upgrade --install $(REDPANDA_RELEASE) $(REDPANDA_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(REDPANDA_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,redpanda) \
		--wait --timeout 10m

.PHONY: system-redpanda-console
system-redpanda-console: vpn-up kube-ctx
	@$(call _helm_repo_add,redpanda,https://charts.redpanda.com)
	$(call _apply_optional_system_secrets,redpanda-console)
	helm upgrade --install $(REDPANDA_CONSOLE_RELEASE) $(REDPANDA_CONSOLE_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(REDPANDA_CONSOLE_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,redpanda-console) \
		--wait --timeout 5m

# ── Observability (LGTM) ────────────────────────────────────────────────
# Loki (store) + Alloy (DaemonSet collector) + Grafana (UI). Install order
# matters: Loki first (Alloy/Grafana point at it), then Alloy, then
# Grafana. `make system` honours that ordering via the chain above.
# No required creds in the baseline (single-tenant, no auth); the optional-
# secrets helper picks up a sealed `grafana-creds` if an env adds one.
.PHONY: system-loki
system-loki: vpn-up kube-ctx
	@$(call _helm_repo_add,grafana,https://grafana.github.io/helm-charts)
	$(call _apply_optional_system_secrets,loki)
	helm upgrade --install $(LOKI_RELEASE) $(LOKI_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(LOKI_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,loki) \
		--wait --timeout 10m

.PHONY: system-alloy
system-alloy: vpn-up kube-ctx
	@$(call _helm_repo_add,grafana,https://grafana.github.io/helm-charts)
	$(call _apply_optional_system_secrets,alloy)
	helm upgrade --install $(ALLOY_RELEASE) $(ALLOY_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(ALLOY_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,alloy) \
		--wait --timeout 5m

.PHONY: system-grafana
system-grafana: vpn-up kube-ctx
	@$(call _helm_repo_add,grafana,https://grafana.github.io/helm-charts)
	$(call _apply_optional_system_secrets,grafana)
	helm upgrade --install $(GRAFANA_RELEASE) $(GRAFANA_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(GRAFANA_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,grafana) \
		--wait --timeout 5m

# Airbyte: helm-install the upstream chart, then run the post-install
# setup wizard via the Airbyte API.
#
# AIRBYTE_SETUP_EMAIL and AIRBYTE_SETUP_ORG are required by the wizard;
# unset them and the script will refuse with a clear error.
.PHONY: system-airbyte
system-airbyte: vpn-up kube-ctx
	@# Preflight the setup-wizard env vars BEFORE the 15-minute helm install
	@# so we don't burn time only to fail on a missing email at the end.
	@test -n "$${AIRBYTE_SETUP_EMAIL:-}" \
		|| { echo "$(C_RED)ERROR$(C_RST): AIRBYTE_SETUP_EMAIL is required for the post-install setup wizard."; \
		     echo "       Re-run with:"; \
		     echo "         AIRBYTE_SETUP_EMAIL=admin@example.com AIRBYTE_SETUP_ORG=Insight \\"; \
		     echo "           make system-airbyte ENV=$(ENV)"; \
		     exit 1; }
	@test -n "$${AIRBYTE_SETUP_ORG:-}" \
		|| { echo "$(C_RED)ERROR$(C_RST): AIRBYTE_SETUP_ORG is required for the post-install setup wizard."; \
		     echo "       Re-run with AIRBYTE_SETUP_ORG=<workspace-name> ..."; \
		     exit 1; }
	@$(call _helm_repo_add,airbyte,https://airbytehq.github.io/helm-charts)
	$(call _apply_optional_system_secrets,airbyte)
	helm upgrade --install $(AIRBYTE_RELEASE) airbyte/airbyte \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(AIRBYTE_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,airbyte) \
		--wait --timeout 15m
	@NAMESPACE=$(NS_INFRA) AIRBYTE_RELEASE=$(AIRBYTE_RELEASE) \
		AIRBYTE_SETUP_EMAIL="$$AIRBYTE_SETUP_EMAIL" \
		AIRBYTE_SETUP_ORG="$$AIRBYTE_SETUP_ORG" \
		bash scripts/airbyte-setup.sh

# Argo Workflows: install the chart with workflowNamespaces+instanceID
# scoped to NS_INFRA, then apply the supplemental RBAC templated for
# $(NS_INFRA).
.PHONY: system-argo
system-argo: vpn-up kube-ctx
	@$(call _helm_repo_add,argo,https://argoproj.github.io/argo-helm)
	$(call _apply_optional_system_secrets,argo-workflows)
	helm upgrade --install $(ARGO_RELEASE) argo/argo-workflows \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(ARGO_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,argo-workflows) \
		--set controller.workflowNamespaces[0]=$(NS_INFRA) \
		--set controller.instanceID.enabled=true \
		--set controller.instanceID.explicitID=$(ARGO_RELEASE)-$(NS_INFRA) \
		--wait --timeout 5m
	@command -v envsubst >/dev/null \
		|| { echo "$(C_RED)ERROR$(C_RST): envsubst not found (install gettext)"; exit 1; }
	@NAMESPACE=$(NS_INFRA) WORKFLOW_SA=$(ARGO_WORKFLOW_SA) \
		envsubst < $(ARGO_RBAC_TMPL) \
		| kubectl apply -n $(NS_INFRA) -f -

.PHONY: system-status
system-status: vpn-up kube-ctx
	@echo "=== Helm releases in $(NS_INFRA) ==="
	@helm list -n $(NS_INFRA) $(HELM_EXTRA_FLAGS) 2>/dev/null || echo "(namespace $(NS_INFRA) absent — run 'make bootstrap ENV=$(ENV)')"
	@echo
	@echo "=== Pods in $(NS_INFRA) ==="
	@kubectl -n $(NS_INFRA) get pods -o wide 2>/dev/null || true

.PHONY: rollback
rollback: vpn-up kube-ctx
	@helm rollback $(RELEASE) --namespace $(NAMESPACE) $(HELM_EXTRA_FLAGS)
	@helm history $(RELEASE) --namespace $(NAMESPACE) --max 5 $(HELM_EXTRA_FLAGS)

.PHONY: status
status: vpn-up kube-ctx
	@helm status $(RELEASE) --namespace $(NAMESPACE) --show-resources $(HELM_EXTRA_FLAGS) || true
	@echo "---"
	@kubectl -n $(NAMESPACE) rollout status deploy --timeout=30s || true

.PHONY: tag
tag: sync-clean
	@TAG="deploy-$$(date -u +%Y.%m.%d.%H.%M)-$$(git rev-parse --short=7 HEAD)"; \
	git tag "$$TAG"; \
	git push origin "$$TAG"; \
	echo "tagged: $$TAG"

# ─── Sealed secrets ────────────────────────────────────────────────────
#
# Default flow: `make seal-secret ENV=local NAMESPACE=insight NAME=insight-db-creds`
# fetches the cleartext Secret YAML by calling `scripts/secret-fetch.sh`
# with the resource name `insight-$(ENV)-$(NAME)`, then pipes the output
# through kubeseal.
#
# `scripts/secret-fetch.sh` ships as a STUB: it reads from a local YAML
# file (secrets-store.yaml.template → copy to secrets-store.yaml) keyed
# by the resource name. Replace it with your own password-manager /
# vault / KMS integration — anything that prints a Kubernetes Secret
# manifest to stdout will work.
#
# Override the resource name with SECRET_NAME=… Or skip the script
# entirely with VALUE_FILE=… (one-off bootstrapping; the cleartext file
# MUST be outside this repo and deleted after). Or pipe a manifest on
# stdin with `make seal-stdin …`.

PUB_CERT := environments/$(ENV)/pub-cert.pem

.PHONY: secret-fetch-present
secret-fetch-present:
	@test -x scripts/secret-fetch.sh \
		|| { echo "$(C_RED)ERROR$(C_RST): scripts/secret-fetch.sh not found or not executable"; \
		     echo "       The shipped stub reads from secrets-store.yaml (copy from"; \
		     echo "       secrets-store.yaml.template). Replace it with an integration"; \
		     echo "       against your password manager for non-sandbox envs."; \
		     exit 1; }

.PHONY: seal
seal: inventory-present secret-fetch-present pub-cert-present
	@echo "$(C_GRN)→$(C_RST) seal infra secrets (namespace=$(NS_INFRA))"
	@yq -r '.secrets.infra[]?    | select(.enabled) | .name' $(INVENTORY) | while read -r name; do \
		[ -z "$$name" ] && continue; \
		echo "  - $$name"; \
		$(MAKE) --no-print-directory seal-secret \
			ENV=$(ENV) NAMESPACE=$(NS_INFRA) NAME=$$name || exit 1; \
	done
	@echo "$(C_GRN)→$(C_RST) seal services secrets (namespace=$(NS_APP))"
	@yq -r '.secrets.services[]? | select(.enabled) | .name' $(INVENTORY) | while read -r name; do \
		[ -z "$$name" ] && continue; \
		echo "  - $$name"; \
		$(MAKE) --no-print-directory seal-secret \
			ENV=$(ENV) NAMESPACE=$(NS_APP) NAME=$$name || exit 1; \
	done
	@echo "$(C_GRN)seal complete$(C_RST) → environments/$(ENV)/sealed-secrets/"

.PHONY: pub-cert-present
pub-cert-present:
	@test -f $(PUB_CERT) \
		|| { echo "$(C_RED)ERROR$(C_RST): pub cert missing at $(PUB_CERT)"; \
		     echo "       run: make fetch-cert ENV=$(ENV)"; exit 1; }

.PHONY: seal-secret
seal-secret: secret-fetch-present pub-cert-present
	@test -n "$(NAMESPACE)" || { echo "NAMESPACE is required"; exit 1; }
	@test -n "$(NAME)"      || { echo "NAME is required"; exit 1; }
	@set -o pipefail; \
	RES_NAME="$${SECRET_NAME:-insight-$(ENV)-$(NAME)}"; \
	mkdir -p environments/$(ENV)/sealed-secrets/$(NAMESPACE); \
	OUT=environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-sealedsecret.yaml; \
	TMP="$$OUT.tmp.$$$$"; \
	echo "fetching secret: $$RES_NAME"; \
	if scripts/secret-fetch.sh "$$RES_NAME" \
	     | kubeseal --format yaml --cert $(PUB_CERT) \
	     > "$$TMP"; then \
	  mv "$$TMP" "$$OUT"; \
	  echo "$(C_GRN)wrote$(C_RST) → $$OUT"; \
	else \
	  rc=$$?; rm -f "$$TMP"; \
	  echo "$(C_RED)ERROR$(C_RST): seal failed (exit $$rc); $$OUT not written"; \
	  exit $$rc; \
	fi

# Fallback: read the cleartext Secret YAML from a local file (e.g. one
# saved temporarily from `kubectl get secret X -o yaml`). The file is
# NOT touched by this target; delete it manually after sealing.
.PHONY: seal-from-file
seal-from-file: pub-cert-present
	@test -n "$(NAMESPACE)"  || { echo "NAMESPACE is required"; exit 1; }
	@test -n "$(NAME)"       || { echo "NAME is required"; exit 1; }
	@test -n "$(VALUE_FILE)" || { echo "VALUE_FILE is required"; exit 1; }
	@test -f "$(VALUE_FILE)" || { echo "VALUE_FILE not found: $(VALUE_FILE)"; exit 1; }
	@mkdir -p environments/$(ENV)/sealed-secrets/$(NAMESPACE)
	@OUT=environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-sealedsecret.yaml; \
	cat "$(VALUE_FILE)" \
	  | kubeseal --format yaml --cert $(PUB_CERT) \
	  > $$OUT; \
	echo "$(C_GRN)wrote$(C_RST) → $$OUT"

# Fallback: read the cleartext Secret YAML from stdin (Ctrl-D to end).
.PHONY: seal-stdin
seal-stdin: pub-cert-present
	@test -n "$(NAMESPACE)" || { echo "NAMESPACE is required"; exit 1; }
	@test -n "$(NAME)"      || { echo "NAME is required"; exit 1; }
	@mkdir -p environments/$(ENV)/sealed-secrets/$(NAMESPACE)
	@OUT=environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-sealedsecret.yaml; \
	echo "reading cleartext Secret manifest from stdin (Ctrl-D to end)…"; \
	kubeseal --format yaml --cert $(PUB_CERT) > $$OUT; \
	echo "$(C_GRN)wrote$(C_RST) → $$OUT"

.PHONY: seal-from-template
seal-from-template: pub-cert-present
	@test -n "$(NAMESPACE)" || { echo "NAMESPACE is required"; exit 1; }
	@test -n "$(NAME)"      || { echo "NAME is required"; exit 1; }
	@TPL=environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-secret-template.yaml; \
	test -f $$TPL || { echo "template not found: $$TPL"; exit 1; }; \
	OUT=environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-sealedsecret.yaml; \
	kubeseal --format yaml --cert $(PUB_CERT) < $$TPL > $$OUT; \
	echo "$(C_GRN)wrote$(C_RST) → $$OUT"

.PHONY: clear-seal-template
clear-seal-template:
	@test -n "$(NAMESPACE)" || { echo "NAMESPACE is required"; exit 1; }
	@test -n "$(NAME)"      || { echo "NAME is required"; exit 1; }
	@TPL=environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-secret-template.yaml; \
	test -f $$TPL || { echo "template not found: $$TPL"; exit 1; }; \
	yq -i '(.stringData // .data) |= with_entries(.value = "")' $$TPL; \
	echo "cleared values in $$TPL"

.PHONY: fetch-cert
fetch-cert: vpn-up kube-ctx
	@kubeseal --fetch-cert --context $(KUBE_CTX) > $(PUB_CERT)
	@echo "$(C_GRN)wrote$(C_RST) → $(PUB_CERT)"

# ─── Cluster bootstrap ────────────────────────────────────────────────
#
# One-time install of cluster prerequisites (ingress controller,
# cert-manager, sealed-secrets-controller) for an env. Idempotent: a
# re-run is a no-op apart from helm-upgrade noise.
#
# Per-env values live at bootstrap/<ENV>/*.yaml. Defaults loaded only
# if the file exists, so an env with no overrides falls back to chart
# defaults.

.PHONY: bootstrap
bootstrap: inventory-present vpn-up kube-ctx
	$(call _run_if_enabled,bootstrap.namespaces,bootstrap-namespaces)
	$(call _run_if_enabled,bootstrap.ingressNginx,bootstrap-ingress-nginx)
	$(call _run_if_enabled,bootstrap.certManager,bootstrap-cert-manager)
	$(call _run_if_enabled,bootstrap.sealedSecrets,bootstrap-sealed-secrets)
	@echo "$(C_GRN)bootstrap complete$(C_RST) → $(KUBE_CTX)"

# Bootstrap is the only place that creates the
# L2/L3 namespaces. Idempotent — `kubectl create ns --dry-run | apply`.
.PHONY: bootstrap-namespaces
bootstrap-namespaces: vpn-up kube-ctx
	@for ns in $(NS_INFRA) $(NS_APP); do \
		kubectl --context $(KUBE_CTX) create namespace "$$ns" \
			--dry-run=client -o yaml \
		| kubectl --context $(KUBE_CTX) apply -f - >/dev/null; \
		echo "$(C_GRN)namespace$(C_RST) → $$ns"; \
	done

.PHONY: bootstrap-ingress-nginx
bootstrap-ingress-nginx: vpn-up kube-ctx
	@helm repo add ingress-nginx https://kubernetes.github.io/ingress-nginx >/dev/null 2>&1 || true
	@helm repo update ingress-nginx >/dev/null
	@VALUES_ARGS=""; \
	if [ -s $(BOOTSTRAP_DIR)/ingress-nginx-values.yaml ]; then \
		VALUES_ARGS="-f $(BOOTSTRAP_DIR)/ingress-nginx-values.yaml"; \
	fi; \
	helm upgrade --install $(INGRESS_NGINX_RELEASE) ingress-nginx/ingress-nginx \
		--namespace $(INGRESS_NGINX_NAMESPACE) --create-namespace \
		--version $(INGRESS_NGINX_VERSION) $(HELM_EXTRA_FLAGS) \
		$$VALUES_ARGS \
		--wait --timeout 5m

.PHONY: bootstrap-cert-manager
bootstrap-cert-manager: vpn-up kube-ctx
	@helm repo add jetstack https://charts.jetstack.io >/dev/null 2>&1 || true
	@helm repo update jetstack >/dev/null
	@VALUES_ARGS=""; \
	if [ -s $(BOOTSTRAP_DIR)/cert-manager-values.yaml ]; then \
		VALUES_ARGS="-f $(BOOTSTRAP_DIR)/cert-manager-values.yaml"; \
	fi; \
	helm upgrade --install $(CERT_MANAGER_RELEASE) jetstack/cert-manager \
		--namespace $(CERT_MANAGER_NAMESPACE) --create-namespace \
		--version $(CERT_MANAGER_VERSION) $(HELM_EXTRA_FLAGS) \
		--set crds.enabled=true \
		$$VALUES_ARGS \
		--wait --timeout 5m
	@kubectl --context $(KUBE_CTX) -n $(CERT_MANAGER_NAMESPACE) \
		rollout status deploy/$(CERT_MANAGER_RELEASE)-webhook --timeout=60s
	@if [ -f $(BOOTSTRAP_DIR)/selfsigned-issuer.yaml ]; then \
		echo "applying $(BOOTSTRAP_DIR)/selfsigned-issuer.yaml"; \
		kubectl --context $(KUBE_CTX) apply -f $(BOOTSTRAP_DIR)/selfsigned-issuer.yaml; \
	fi

.PHONY: bootstrap-sealed-secrets
bootstrap-sealed-secrets: vpn-up kube-ctx
	@# The old bitnami-labs.github.io/sealed-secrets host now returns
	@# 404. Bitnami consolidated the index to bitnami.github.io (note
	@# the -labs is gone). If the repo was registered under the old
	@# URL on this machine, drop it first so `repo add` re-registers
	@# the new host instead of silently keeping the dead one.
	@OLD_REPOS=$$(helm repo list 2>/dev/null | awk '$$1=="sealed-secrets" && $$2!="https://bitnami.github.io/sealed-secrets" {print $$1}'); \
	if [ -n "$$OLD_REPOS" ]; then \
		printf '%s\n' "$$OLD_REPOS" | xargs -n1 helm repo remove >/dev/null 2>&1 || true; \
	fi
	@helm repo add sealed-secrets https://bitnami.github.io/sealed-secrets >/dev/null 2>&1 || true
	@helm repo update sealed-secrets >/dev/null
	@VALUES_ARGS=""; \
	if [ -s $(BOOTSTRAP_DIR)/sealed-secrets-values.yaml ]; then \
		VALUES_ARGS="-f $(BOOTSTRAP_DIR)/sealed-secrets-values.yaml"; \
	fi; \
	helm upgrade --install $(SEALED_SECRETS_RELEASE) sealed-secrets/sealed-secrets \
		--namespace $(SEALED_SECRETS_NAMESPACE) \
		--version $(SEALED_SECRETS_VERSION) $(HELM_EXTRA_FLAGS) \
		$$VALUES_ARGS \
		--wait --timeout 3m

.PHONY: bootstrap-status
bootstrap-status: vpn-up kube-ctx
	@echo "=== ingress-nginx ==="
	@helm --kube-context $(KUBE_CTX) -n $(INGRESS_NGINX_NAMESPACE) status $(INGRESS_NGINX_RELEASE) 2>/dev/null | head -3 || echo "(not installed)"
	@kubectl --context $(KUBE_CTX) get ingressclass 2>/dev/null
	@echo
	@echo "=== cert-manager ==="
	@helm --kube-context $(KUBE_CTX) -n $(CERT_MANAGER_NAMESPACE) status $(CERT_MANAGER_RELEASE) 2>/dev/null | head -3 || echo "(not installed)"
	@kubectl --context $(KUBE_CTX) get clusterissuer 2>/dev/null
	@echo
	@echo "=== sealed-secrets ==="
	@helm --kube-context $(KUBE_CTX) -n $(SEALED_SECRETS_NAMESPACE) status $(SEALED_SECRETS_RELEASE) 2>/dev/null | head -3 || echo "(not installed)"
