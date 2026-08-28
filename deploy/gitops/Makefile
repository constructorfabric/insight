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
#   make bootstrap ENV=<env>                       # L0: envoy-gateway, cert-manager, sealed-secrets, namespaces
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

# bash, not /bin/sh: `deploy-insight` needs `set -o pipefail` to read helm's
# exit status through `tee`; dash lacks it and a failed upgrade reports SUCCESS.
SHELL            := /bin/bash

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
# Covers rollout waits plus the post-upgrade clickhouse-migrate hook; the
# gold build inside that hook spills to disk under memory pressure, which
# trades speed for reliability — give it room before --atomic rolls back.
TIMEOUT          ?= 30m
RENDER_DIR       := .deploy

# yq trap: `//` treats an explicit `false` as absent, so CREATE_NS/ATOMIC
# carry no fallback and read a missing key back as the literal "null".
CREATE_NS        ?= $(shell yq -r '.bootstrap.namespaces' $(INVENTORY) 2>/dev/null)
ATOMIC           ?= $(shell yq -r '.deploy.atomic'        $(INVENTORY) 2>/dev/null)

KEYCLOAK_REALMS  := $(shell yq -r '.keycloak.realms'      $(INVENTORY) 2>/dev/null)
SEALED_ENABLED   := $(shell yq -r '[.secrets.services[]? | select(.enabled == true)] | length' $(INVENTORY) 2>/dev/null)
OIDC_SECRET      := $(shell yq -r '.oidcClientSecret.secret // ""' $(INVENTORY) 2>/dev/null)
OIDC_SECRET_KEY  := $(shell yq -r '.oidcClientSecret.key // "client-secret"' $(INVENTORY) 2>/dev/null)
# Read from VALUES, not the inventory: the chart is told the mode there, and
# two copies can disagree.
CRED_MODE        := $(shell yq -r '.credentials.deploymentMode' $(VALUES) 2>/dev/null)

HELM_NS_FLAG     := $(if $(filter false,$(CREATE_NS)),,--create-namespace)
# `--atomic` rolls back to the PREVIOUS chart and deletes the pods holding the
# failure's evidence — a downgrade here. `deploy.atomic: false` turns it off.
HELM_ATOMIC_FLAG := $(if $(filter false,$(ATOMIC)),,--atomic)

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

# Airbyte 1.8+ merged the webapp service into server and stopped publishing
# a separate webapp image — UI is served by the server service. Bumping below
# 1.8 (or rolling back) requires re-adding the webapp subchart/routing that
# system/airbyte/values.yaml removed; see issue #1910.
AIRBYTE_RELEASE  ?= airbyte
AIRBYTE_VERSION  ?= 1.9.2

ARGO_RELEASE     ?= argo-workflows
ARGO_VERSION     ?= 1.0.13
ARGO_RBAC_TMPL   ?= bootstrap/argo-rbac.yaml.template
ARGO_WORKFLOW_SA ?= argo-workflow

# Observability stack (LGTM). Installed only when the bundled stack is
# enabled via inventory.system.{victoriametrics,loki,alloy,grafana};
# otherwise the host cluster collects from stdout and these targets are
# skipped.
# Chart versions verified latest via `helm search repo <repo>/<chart>
# --versions` (bump deliberately, same discipline as the pins above).
LOKI_RELEASE     ?= loki
# grafana/loki now tracks Grafana Enterprise Logs; the OSS chart continues
# as grafana-community/loki (same manifests, renumbered versions).
LOKI_CHART       ?= grafana-community/loki
LOKI_VERSION     ?= 18.11.4    # app 3.7.7

ALLOY_RELEASE    ?= alloy
ALLOY_CHART      ?= grafana/alloy
ALLOY_VERSION    ?= 1.12.1     # app v1.19.2

GRAFANA_RELEASE  ?= grafana
# grafana/grafana is deprecated (frozen 2026-01-30); development moved to
# grafana-community/helm-charts. Alloy still lives in the old repo.
GRAFANA_CHART    ?= grafana-community/grafana
GRAFANA_VERSION  ?= 13.0.0     # app 13.2.0

# Metrics store — PromQL-compatible, remote-write receiver built in at
# /api/v1/write (always on, no flag needed).
VICTORIAMETRICS_RELEASE ?= victoriametrics
VICTORIAMETRICS_CHART   ?= vm/victoria-metrics-single
VICTORIAMETRICS_VERSION ?= 0.45.0

# Per-cluster bootstrap controllers (gateway, cert-manager, sealed-secrets).
# Versions pinned here; bump deliberately. Per-env overrides go in
# bootstrap/<env>/<controller>-values.yaml. Namespaces fall through to
# inventory.namespaces.<controller> for rare-but-allowed overrides.
ENVOY_GATEWAY_NAMESPACE  ?= $(shell yq -r '.namespaces.envoyGateway  // "envoy-gateway-system"' $(INVENTORY) 2>/dev/null)
ENVOY_GATEWAY_RELEASE    ?= envoy-gateway
ENVOY_GATEWAY_VERSION    ?= 1.8.3

CERT_MANAGER_NAMESPACE   ?= $(shell yq -r '.namespaces.certManager   // "cert-manager"' $(INVENTORY) 2>/dev/null)
CERT_MANAGER_RELEASE     ?= cert-manager
CERT_MANAGER_VERSION     ?= 1.20.2
# The gateway-shim needs the Gateway API CRDs already present to start, so an
# env whose cluster has none must switch it off or the controller never comes
# up. Defaults on — an env that terminates TLS at a Gateway wants the shim.
# Which component ships those CRDs, and in what order: test-stand/INFRA.md.
# Defaulted in make rather than yq's `//`, which falls through on an explicit
# `false` as readily as on a missing key and would swallow every override.
CERT_MANAGER_GATEWAY_SHIM ?= $(if $(filter false,$(shell yq -r '.bootstrap.certManagerGatewayShim' $(INVENTORY) 2>/dev/null)),false,true)

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
	@echo "  make print-confirm-token ENV=$(ENV)  Print the CONFIRM token 'make deploy' expects — the one source of truth"
	@echo "  make apply-app-secrets ENV=$(ENV)   L3 prereq — apply sealed manifests + derive *-config Secrets from insight-db-creds"
	@echo "  make system ENV=$(ENV)            L2 — install all services with inventory.system.<svc>: true"
	@echo "  make system-mariadb ENV=$(ENV)            }"
	@echo "  make system-clickhouse ENV=$(ENV)         } L2 — one-at-a-time for one-offs / rotation."
	@echo "  make system-redis ENV=$(ENV)              } Top-level 'make system' chains all enabled in inventory."
	@echo "  make system-redpanda ENV=$(ENV)           }"
	@echo "  make system-redpanda-console ENV=$(ENV)   }"
	@echo "  make system-airbyte ENV=$(ENV)            }   (Airbyte = chart + post-install setup wizard)"
	@echo "  make system-argo ENV=$(ENV)               }   (Argo Workflows = chart + supplemental RBAC apply)"
	@echo "  make system-victoriametrics ENV=$(ENV)    }"
	@echo "  make system-loki ENV=$(ENV)               }"
	@echo "  make system-alloy ENV=$(ENV)              } L2 — observability (LGTM, bundled mode)."
	@echo "  make system-grafana ENV=$(ENV)            }"
	@echo "  make system-status ENV=$(ENV)             Show what's installed in insight-infra"
	@echo "  make provision-ci ENV=$(ENV)      Provision the namespace-scoped CI deploy credential"
	@echo "  make verify-ci-credential ENV=$(ENV)  Assert the CI credential's RBAC scope"
	@echo "  make revoke-ci ENV=$(ENV)         Delete the CI deploy credential (SA + RBAC + token)"
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
	@# Scoped to THIS directory: the guarantee is that what is about to be
	@# deployed is committed, not that the whole repository is clean.
	@if [ -n "$$(git status --porcelain -- .)" ]; then \
		echo "$(C_RED)ERROR$(C_RST): deploy/gitops has uncommitted changes"; \
		git status --short -- .; exit 1; fi
	@# Fetch sits inside the upstream test: a detached CI HEAD has no upstream.
	@if ! git rev-parse --abbrev-ref @{u} >/dev/null 2>&1; then \
		echo "$(C_YEL)WARN$(C_RST): no upstream tracked; skipping HEAD-vs-remote check"; \
	else \
		git fetch --quiet origin; \
		LOCAL=$$(git rev-parse @); REMOTE=$$(git rev-parse @{u}); \
		if [ "$$LOCAL" != "$$REMOTE" ]; then \
			echo "$(C_RED)ERROR$(C_RST): HEAD is not equal to origin/$$(git rev-parse --abbrev-ref @{u} | cut -d/ -f2-)"; \
			echo "       run 'make sync' first"; exit 1; \
		fi; \
	fi

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
	@# `auth can-i --list`, not `cluster-info`: cluster-info reads kube-system,
	@# which a namespace-scoped credential is refused on a reachable cluster.
	@kubectl --context $(KUBE_CTX) --request-timeout=5s auth can-i --list -n $(NS_APP) >/dev/null 2>&1 \
		|| { echo "$(C_RED)ERROR$(C_RST): cannot reach cluster '$(KUBE_CTX)' as an authenticated principal;"; \
		     echo "       is the VPN up, and is the kubeconfig's credential still valid? On a namespace-scoped"; \
		     echo "       CI credential, re-issue it with scripts/provision-ci-deployer.sh."; exit 1; }

# The actual current context is not printed on a mismatch: an arbitrary string
# is not credential-shaped, so a public CI log's redactor would not catch it.
.PHONY: kube-ctx
kube-ctx: kube-ctx-required
	@CUR=$$(kubectl config current-context 2>/dev/null || echo none); \
	if [ "$$CUR" != "$(KUBE_CTX)" ]; then \
		echo "$(C_RED)ERROR$(C_RST): the active kube-context is not '$(KUBE_CTX)' ($(INVENTORY)'s kubeContext)."; \
		echo "       Either the credential in use points at the wrong cluster, or the inventory was"; \
		echo "       changed without rotating it — run: kubectl config use-context $(KUBE_CTX)"; \
		exit 1; fi

# Single source of truth for the deploy CONFIRM token. `confirm` below and
# `make -s print-confirm-token` consume it; nothing re-derives the format.
CONFIRM_TOKEN := yes-deploy-$(ENV)

.PHONY: confirm
confirm: inventory-present
	@if [ "$$(yq -r '.protected // false' $(INVENTORY))" = "true" ]; then \
		if [ "$(CONFIRM)" != "$(CONFIRM_TOKEN)" ]; then \
			echo "$(C_RED)ERROR$(C_RST): deploy to '$(ENV)' requires CONFIRM=$(CONFIRM_TOKEN) (per inventory.protected)"; \
			exit 1; fi; \
	fi

.PHONY: print-confirm-token
print-confirm-token:
	@echo "$(CONFIRM_TOKEN)"

.PHONY: chart-present
chart-present:
	@test -n "$(INSIGHT_VERSION)" \
		|| { echo "$(C_RED)ERROR$(C_RST): INSIGHT_VERSION is empty; populate .insight-version"; exit 1; }
	@helm show chart $(CHART) --version $(INSIGHT_VERSION) $(HELM_EXTRA_FLAGS) >/dev/null 2>&1 \
		|| { echo "$(C_RED)ERROR$(C_RST): chart $(CHART):$(INSIGHT_VERSION) not found in registry"; \
		     echo "       publish-chart is what puts it there; if that job failed after pushing but"; \
		     echo "       before committing the version bump, re-run it rather than deploying by hand."; \
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
# ENV=local sandbox inputs for the in-stack Keycloak: generate the roster realm
# into the env realms dir — keycloak-broker-realms packs it with the rest — and
# ensure the sandbox admin/placeholder Secret. No-op unless keycloak.deploy=true.
#
# The generator lives in the seed package (it builds the realm from the same
# roster the database seed writes) and runs as its installed entry point, so
# this recipe needs uv. It also needs the tenant: the generator requires it
# rather than defaulting, because a realm whose tenant claim differs from the
# seeded rows authenticates users that then resolve to nobody.
keycloak-realm: kube-ctx values-present
	@set -e; \
	if [ "$$(yq -r '.keycloak.deploy // false' $(VALUES))" != "true" ]; then \
	  echo "$(C_YEL)skip$(C_RST) keycloak-realm (keycloak.deploy != true)"; exit 0; fi; \
	SEED_DIR="$$(cd ../.. && pwd)/src/ingestion/tools/seed"; \
	command -v uv >/dev/null 2>&1 || { echo "$(C_RED)uv is required to generate the Keycloak realm$(C_RST)"; exit 1; }; \
	DIR=environments/$(ENV)/keycloak/realms; mkdir -p "$$DIR"; \
	OUT="$$DIR/realm-insight.generated.json"; \
	DEV_EMAIL="$$(yq -r '.keycloak.devUserEmail // "dev@company.nonpresent"' $(VALUES))"; \
	REDIRECT="$$(yq -r '.authenticator.oidc.redirectUri // "http://localhost/auth/callback"' $(VALUES))"; \
	AUTH_SECRET="$$(yq -r '.authenticator.oidc.clientSecret // "insight-authenticator-dev-secret"' $(VALUES))"; \
	KC_BASE="$$(yq -r '.keycloak.hostname // ""' $(VALUES))"; \
	LB_REDIRECT="$${KC_BASE%/kc}/auth/callback"; \
	echo "$(C_GRN)→$(C_RST) generating Keycloak realm (dev-lead $$DEV_EMAIL; redirects $$REDIRECT, $$LB_REDIRECT)"; \
	TENANT_ID="$$(yq -r '.global.tenantDefaultId // ""' $(VALUES))"; \
	[ -n "$$TENANT_ID" ] || { echo "$(C_RED)global.tenantDefaultId is required to generate the realm$(C_RST)"; exit 1; }; \
	[ -z "$$SEED_ORG_HEADCOUNT" ] || echo "$(C_GRN)→$(C_RST) realm sized for SEED_ORG_HEADCOUNT=$$SEED_ORG_HEADCOUNT people"; \
	SEED_ORG_HEADCOUNT="$$SEED_ORG_HEADCOUNT" TENANT_DEFAULT_ID="$$TENANT_ID" uv run --project "$$SEED_DIR" insight-seed-realm --dev-email "$$DEV_EMAIL" \
	  --authenticator-redirect "$$REDIRECT" \
	  --authenticator-redirect "$$LB_REDIRECT" \
	  --authenticator-secret "$$AUTH_SECRET" \
	  --out "$$OUT"; \
	kubectl --context $(KUBE_CTX) -n $(NS_APP) create secret generic insight-keycloak-config \
	  --from-literal=username=admin --from-literal=password=admin \
	  --from-literal=INSIGHT_AUTHENTICATOR_CLIENT_SECRET="$$AUTH_SECRET" \
	  --from-literal=INSIGHT_AUTHENTICATOR_REDIRECT_URI="$$REDIRECT" \
	  --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) -n $(NS_APP) apply -f -; \
	echo "$(C_GRN)generated$(C_RST) $$OUT + sandbox secret insight-keycloak-config"

.PHONY: keycloak-broker-realms
# Pack environments/$(ENV)/keycloak/realms/* into the ConfigMap the
# umbrella's keycloak-config-cli hook Job applies (ADR-0003). Chained into
# deploy-insight; no-op unless keycloakConfig.enabled=true.
keycloak-broker-realms: kube-ctx values-present
	@# `external` means the realm ConfigMap is written outside this repository.
	@# Keyed on ownership, not on keycloakConfig.enabled.
	@if [ "$(KEYCLOAK_REALMS)" = "external" ]; then \
	  echo "$(C_YEL)skip$(C_RST) keycloak-broker-realms (inventory.keycloak.realms=external — $(RELEASE)-keycloak-config-realms is owned outside this repo)"; exit 0; fi; \
	if [ "$$(yq -r '.keycloakConfig.enabled // false' $(VALUES))" != "true" ]; then \
	  echo "$(C_YEL)skip$(C_RST) keycloak-broker-realms (keycloakConfig.enabled != true)"; exit 0; fi; \
	DIR=environments/$(ENV)/keycloak/realms; \
	ls $$DIR/* >/dev/null 2>&1 \
	  || { echo "$(C_RED)ERROR$(C_RST): keycloakConfig.enabled=true but no realm files in $$DIR/"; exit 1; }; \
	kubectl --context $(KUBE_CTX) -n $(NS_APP) create configmap $(RELEASE)-keycloak-config-realms \
	  --from-file="$$DIR" --dry-run=client -o yaml | \
	  kubectl --context $(KUBE_CTX) -n $(NS_APP) apply -f -; \
	echo "$(C_GRN)applied$(C_RST) configmap $(RELEASE)-keycloak-config-realms → $(NS_APP)"

.PHONY: deploy-app
deploy-app: deploy-insight

# Apply L3 sealed secrets to the cluster + derive the composed
# *-config Secrets that the chart references via `existingSecret`.
# Derived Secrets are NOT sealed/committed — they're regenerated from
# the already-sealed insight-db-creds on every deploy. Idempotent.
#
# Two independent skips: no enabled sealed Secret in the inventory, and
# CRED_MODE=helm — composing over the chart can write a BLANK client secret.
.PHONY: apply-app-secrets
apply-app-secrets: vpn-up kube-ctx
	@if [ "$(SEALED_ENABLED)" = "0" ]; then \
		echo "$(C_YEL)skip$(C_RST) sealed manifests (inventory.secrets.services has no enabled entry — every Secret this release consumes is created outside this repo)"; \
	else \
		SEAL_DIR=environments/$(ENV)/sealed-secrets/insight; \
		if [ ! -d "$$SEAL_DIR" ] || [ -z "$$(ls -A $$SEAL_DIR/*-sealedsecret.yaml 2>/dev/null)" ]; then \
			echo "$(C_RED)ERROR$(C_RST): no L3 sealed manifests in $$SEAL_DIR/"; \
			echo "       seal at minimum insight-db-creds before running deploy-app:"; \
			echo "         make seal-secret ENV=$(ENV) NAMESPACE=$(NS_APP) NAME=insight-db-creds"; \
			exit 1; \
		fi; \
		echo "applying L3 sealed manifests under $$SEAL_DIR/"; \
		for f in $$SEAL_DIR/*-sealedsecret.yaml; do \
			kubectl --context $(KUBE_CTX) apply -f "$$f"; \
		done; \
		echo "waiting for sealed-secrets-controller to materialise insight-db-creds…"; \
		for i in $$(seq 1 30); do \
			if kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret insight-db-creds >/dev/null 2>&1; then \
				break; \
			fi; \
			sleep 1; \
		done; \
		kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret insight-db-creds >/dev/null 2>&1 \
			|| { echo "$(C_RED)ERROR$(C_RST): insight-db-creds never materialised (controller down? sealed manifest invalid?)"; exit 1; }; \
	fi
	@# Tested for `helm`, not against `gitops`: an unreadable values file reads
	@# back EMPTY, and "empty is not gitops" would skip this step for ENV=local.
	@if [ "$(CRED_MODE)" = "helm" ]; then \
		echo "$(C_YEL)skip$(C_RST) compose-app-secrets (credentials.deploymentMode=helm — the chart renders the *-config Secrets)"; \
	else \
		ENV=$(ENV) NS_APP=$(NS_APP) RELEASE=$(RELEASE) bash scripts/compose-app-secrets.sh; \
	fi

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
deploy-insight: sync-clean vpn-up kube-ctx confirm values-present chart-present apply-app-secrets keycloak-broker-realms
	@mkdir -p $(RENDER_DIR)
	@helm template $(RELEASE) $(CHART) --version $(INSIGHT_VERSION) $(HELM_EXTRA_FLAGS) \
		-n $(NS_APP) -f $(VALUES) \
		> $(RENDER_DIR)/last-render-$(ENV).yaml
	@echo "$(C_GRN)rendered$(C_RST) $(CHART):$(INSIGHT_VERSION) → $(RENDER_DIR)/last-render-$(ENV).yaml"
	@# A release whose only revision is a failed install cannot be upgraded, and
	@# a `pending-*` one is refused rather than auto-recovered. See INFRA.md.
	@set -o pipefail; \
	STATUS=$$(helm list -n $(NS_APP) --failed --pending --uninstalling --filter '^$(RELEASE)$$' -o json $(HELM_EXTRA_FLAGS) 2>/dev/null | jq -r '.[0].status // ""'); \
	case "$$STATUS" in \
	  failed) \
	    if ! helm history $(RELEASE) -n $(NS_APP) -o json $(HELM_EXTRA_FLAGS) 2>/dev/null | jq -e 'any(.[]; .status == "deployed" or .status == "superseded")' >/dev/null; then \
	      echo "$(C_RED)ERROR$(C_RST): release '$(RELEASE)' has no deployed revision — its only history is a failed install."; \
	      echo "       helm refuses to upgrade such a release ('has no deployed releases'), and will keep"; \
	      echo "       refusing. Remove it and let this target install cleanly:"; \
	      echo "         helm uninstall $(RELEASE) -n $(NS_APP)"; \
	      exit 1; \
	    fi ;; \
	  pending-install|pending-upgrade|pending-rollback) \
	    echo "$(C_RED)ERROR$(C_RST): release '$(RELEASE)' is '$$STATUS' — a previous helm was killed mid-flight."; \
	    echo "       Another operation is in progress as far as helm is concerned, and this upgrade"; \
	    echo "       would be refused. Check nothing is still running, then:"; \
	    echo "         helm rollback $(RELEASE) -n $(NS_APP)   # to the last deployed revision, or"; \
	    echo "         helm uninstall $(RELEASE) -n $(NS_APP)  # if the stand is disposable"; \
	    exit 1 ;; \
	esac
	@# `--wait` is explicit, not inherited from `--atomic`: on helm 4 an omitted
	@# `--wait` waits on hooks only. `--set-file`, not `--set-string`, which
	@# splits on commas and backslashes and would truncate a generated secret.
	@set -o pipefail; \
	SET_ARGS=(); \
	if [ -n "$(OIDC_SECRET)" ]; then \
		OIDC_FILE=$$(mktemp); trap 'rm -f "$$OIDC_FILE"' EXIT INT TERM; \
		kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret $(OIDC_SECRET) \
			-o "jsonpath={.data['$(OIDC_SECRET_KEY)']}" | base64 --decode > "$$OIDC_FILE"; \
		[ -s "$$OIDC_FILE" ] \
			|| { echo "$(C_RED)ERROR$(C_RST): secret $(OIDC_SECRET) has no non-empty '$(OIDC_SECRET_KEY)' in $(NS_APP)."; \
			     echo "       Refusing to deploy: the upgrade would succeed and leave the authenticator with a"; \
			     echo "       blank client secret, which looks healthy and fails every login."; exit 1; }; \
		SET_ARGS+=(--set-file authenticator.oidc.clientSecret="$$OIDC_FILE"); \
	fi; \
	LOG=$(RENDER_DIR)/deploy-$(ENV)-$$(date -u +%Y%m%d-%H%M%S).log; \
	echo "$(C_GRN)applying$(C_RST) → log at $$LOG"; \
	rc=0; \
	helm upgrade --install $(RELEASE) $(CHART) \
		--version $(INSIGHT_VERSION) $(HELM_EXTRA_FLAGS) $(HELM_UPGRADE_FLAGS) "$${SET_ARGS[@]}" \
		--namespace $(NS_APP) $(HELM_NS_FLAG) \
		-f $(VALUES) \
		--wait $(HELM_ATOMIC_FLAG) --timeout $(TIMEOUT) \
		--history-max 10 \
		2>&1 | tee $$LOG || rc=$$?; \
	if [ "$$(yq -r '.system.loki // false' $(INVENTORY) 2>/dev/null)" = "true" ]; then \
		NS_INFRA=$(NS_INFRA) KUBE_CTX=$(KUBE_CTX) \
			bash scripts/push-deploy-log.sh $(ENV) $$LOG $(INSIGHT_VERSION) || true; \
	fi; \
	exit $$rc
	@$(MAKE) --no-print-directory verify-release ENV=$(ENV) INSIGHT_VERSION=$(INSIGHT_VERSION)

# A release can report `deployed` and still be running the wrong chart.
.PHONY: verify-release
verify-release: vpn-up kube-ctx
	@# Status flags enumerated instead of `--all`, which helm 4 removed: on a v4
	@# host `--all` exits `unknown flag` and reports every release as absent.
	@LISTED=$$(helm list -n $(NS_APP) --deployed --failed --pending --uninstalling --filter '^$(RELEASE)$$' -o json $(HELM_EXTRA_FLAGS)); \
	STATUS=$$(printf '%s' "$$LISTED" | jq -r '.[0].status // "absent"'); \
	CHARTV=$$(printf '%s' "$$LISTED" | jq -r '.[0].chart // "absent"'); \
	REV=$$(printf '%s' "$$LISTED" | jq -r '.[0].revision // "?"'); \
	echo "release $(RELEASE): status=$$STATUS chart=$$CHARTV revision=$$REV"; \
	[ "$$STATUS" = "deployed" ] \
		|| { echo "$(C_RED)ERROR$(C_RST): release is '$$STATUS', not 'deployed' — left in that state on purpose."; exit 1; }; \
	[ "$$CHARTV" = "insight-$(INSIGHT_VERSION)" ] \
		|| { echo "$(C_RED)ERROR$(C_RST): the stand runs '$$CHARTV', not 'insight-$(INSIGHT_VERSION)'."; \
		     echo "       helm reported success while installing something else — treat this as a problem"; \
		     echo "       with the chart reference or the version input, not with the stand."; exit 1; }

# Helper: chain a sub-target IF the matching inventory.<section>.<key>
# is true; otherwise print a skip message. Use with $(call …) below.
#   $(1) — inventory key (e.g. bootstrap.envoyGateway)
#   $(2) — sub-target name (e.g. bootstrap-envoy-gateway)
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
	$(call _run_if_enabled,system.victoriametrics,system-victoriametrics)
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
# VictoriaMetrics (metrics store) + Loki (log store) + Alloy (DaemonSet
# collector) + Grafana (UI). Install order matters: stores first (Alloy/
# Grafana point at them), then Alloy, then Grafana. `make system` honours
# that ordering via the chain above.
# No required creds in the baseline (single-tenant, no auth); the optional-
# secrets helper picks up a sealed `grafana-creds` if an env adds one.
.PHONY: system-victoriametrics
system-victoriametrics: vpn-up kube-ctx
	@$(call _helm_repo_add,vm,https://victoriametrics.github.io/helm-charts/)
	$(call _apply_optional_system_secrets,victoriametrics)
	helm upgrade --install $(VICTORIAMETRICS_RELEASE) $(VICTORIAMETRICS_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(VICTORIAMETRICS_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,victoriametrics) \
		--wait --timeout 10m

.PHONY: system-loki
system-loki: vpn-up kube-ctx
	@$(call _helm_repo_add,grafana-community,https://grafana-community.github.io/helm-charts)
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
	@$(call _helm_repo_add,grafana-community,https://grafana-community.github.io/helm-charts)
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
# scoped to NS_INFRA *and* NS_APP, then apply the supplemental RBAC
# templated for each namespace.
#
# NS_APP is required here too because `ingestion.templates.enabled=true`
# (insight chart) renders the reconcile CronWorkflow/WorkflowTemplates into
# NS_APP, not NS_INFRA — the two-namespace split (services vs. shared infra)
# is this repo's standard topology (see environments/*/inventory.yaml), not
# a one-off. A controller scoped to workflowNamespaces=[NS_INFRA] only can
# never see or run those objects, regardless of RBAC.
.PHONY: system-argo
system-argo: vpn-up kube-ctx
	@$(call _helm_repo_add,argo,https://argoproj.github.io/argo-helm)
	$(call _apply_optional_system_secrets,argo-workflows)
	helm upgrade --install $(ARGO_RELEASE) argo/argo-workflows \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(ARGO_VERSION) $(HELM_EXTRA_FLAGS) \
		$(call _system_values_args,argo-workflows) \
		--set controller.workflowNamespaces[0]=$(NS_INFRA) \
		--set controller.workflowNamespaces[1]=$(NS_APP) \
		--set controller.instanceID.enabled=true \
		--set controller.instanceID.explicitID=$(ARGO_RELEASE)-$(NS_INFRA) \
		--wait --timeout 5m
	@command -v envsubst >/dev/null \
		|| { echo "$(C_RED)ERROR$(C_RST): envsubst not found (install gettext)"; exit 1; }
	@NAMESPACE=$(NS_INFRA) SA_NAMESPACE=$(NS_INFRA) WORKFLOW_SA=$(ARGO_WORKFLOW_SA) \
		envsubst < $(ARGO_RBAC_TMPL) \
		| kubectl apply -n $(NS_INFRA) -f -
	@NAMESPACE=$(NS_APP) SA_NAMESPACE=$(NS_INFRA) WORKFLOW_SA=$(ARGO_WORKFLOW_SA) \
		envsubst < $(ARGO_RBAC_TMPL) \
		| kubectl apply -n $(NS_APP) -f -

.PHONY: system-status
system-status: vpn-up kube-ctx
	@echo "=== Helm releases in $(NS_INFRA) ==="
	@helm list -n $(NS_INFRA) $(HELM_EXTRA_FLAGS) 2>/dev/null || echo "(namespace $(NS_INFRA) absent — run 'make bootstrap ENV=$(ENV)')"
	@echo
	@echo "=== Pods in $(NS_INFRA) ==="
	@kubectl -n $(NS_INFRA) get pods -o wide 2>/dev/null || true

# The namespace-scoped ServiceAccount credential CI deploys with. Procedure:
# environments/test-stand/credentials-runbook.md.
CI_DEPLOYER_MANIFEST     := environments/$(ENV)/ci-deployer-rbac.yaml
CI_DEPLOYER_SA           := ci-deployer
CI_DEPLOYER_TOKEN_SECRET := ci-deployer-token
CI_DEPLOYER_TOKEN_WAIT_S ?= 60
# Context-named after $(KUBE_CTX), not a fixed string: `KUBECONFIG=<out> make
# deploy ENV=$(ENV)` then needs no KUBE_CTX override.
CI_DEPLOYER_OUT          ?= $(HOME)/.kube/insight-$(ENV)-ci-deployer.kubeconfig
CI_DEPLOYER_CA_FILE      ?=
# The chart renders its airbyte-auth-reader Role/RoleBinding into this
# namespace (airbyte.namespace in $(VALUES)), not the release namespace.
AIRBYTE_NAMESPACE        := $(shell yq -r '.airbyte.namespace // ""' $(VALUES) 2>/dev/null)

.PHONY: provision-ci
provision-ci: vpn-up kube-ctx
	@test -f $(CI_DEPLOYER_MANIFEST) \
		|| { echo "$(C_RED)ERROR$(C_RST): no manifest at $(CI_DEPLOYER_MANIFEST)"; exit 1; }
	kubectl --context $(KUBE_CTX) apply -f $(CI_DEPLOYER_MANIFEST)
	@set -o pipefail; \
	echo "waiting for the token controller to populate $(NS_APP)/$(CI_DEPLOYER_TOKEN_SECRET)…"; \
	i=0; FILLED=""; \
	while [ "$$i" -lt $(CI_DEPLOYER_TOKEN_WAIT_S) ]; do \
		FILLED=$$(kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret $(CI_DEPLOYER_TOKEN_SECRET) -o go-template='{{if .data.token}}yes{{end}}' 2>/dev/null); \
		[ -n "$$FILLED" ] && break; \
		i=$$((i + 1)); sleep 1; \
	done; \
	[ -n "$$FILLED" ] || { echo "$(C_RED)ERROR$(C_RST): token controller never populated $(CI_DEPLOYER_TOKEN_SECRET) within $(CI_DEPLOYER_TOKEN_WAIT_S)s"; exit 1; }; \
	OUT_DIR=$$(dirname "$(CI_DEPLOYER_OUT)"); \
	[ -d "$$OUT_DIR" ] || { echo "$(C_RED)ERROR$(C_RST): parent directory of CI_DEPLOYER_OUT does not exist: $$OUT_DIR"; exit 1; }; \
	OUT_ABS="$$OUT_DIR/$$(basename $(CI_DEPLOYER_OUT))"; \
	if git -C "$$OUT_DIR" rev-parse --show-toplevel >/dev/null 2>&1 && ! git -C "$$OUT_DIR" check-ignore -q "$$OUT_ABS" 2>/dev/null; then \
		echo "$(C_RED)ERROR$(C_RST): refusing to write a bearer token to '$$OUT_ABS' — inside a git work tree and not gitignored"; exit 1; \
	fi; \
	TOKEN=$$(kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret $(CI_DEPLOYER_TOKEN_SECRET) -o go-template='{{if .data.token}}{{.data.token | base64decode}}{{end}}' 2>/dev/null); \
	[ -n "$$TOKEN" ] || { echo "$(C_RED)ERROR$(C_RST): token Secret $(CI_DEPLOYER_TOKEN_SECRET) carries no token"; exit 1; }; \
	CLUSTER=$$(kubectl config view -o jsonpath="{.contexts[?(@.name==\"$(KUBE_CTX)\")].context.cluster}"); \
	[ -n "$$CLUSTER" ] || { echo "$(C_RED)ERROR$(C_RST): context $(KUBE_CTX) not found in the active kubeconfig"; exit 1; }; \
	CA_B64=$$(kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret $(CI_DEPLOYER_TOKEN_SECRET) -o jsonpath='{.data.ca\.crt}' 2>/dev/null); \
	[ -n "$$CA_B64" ] || [ -z "$(CI_DEPLOYER_CA_FILE)" ] || CA_B64=$$(base64 < "$(CI_DEPLOYER_CA_FILE)" | tr -d '\n'); \
	[ -n "$$CA_B64" ] || CA_B64=$$(kubectl config view --raw -o jsonpath="{.clusters[?(@.name==\"$$CLUSTER\")].cluster.certificate-authority-data}"); \
	[ -n "$$CA_B64" ] || { echo "$(C_RED)ERROR$(C_RST): could not resolve a cluster CA — set CI_DEPLOYER_CA_FILE=<pem>"; exit 1; }; \
	SERVER=$$(kubectl config view -o jsonpath="{.clusters[?(@.name==\"$$CLUSTER\")].cluster.server}"); \
	[ -n "$$SERVER" ] || { echo "$(C_RED)ERROR$(C_RST): could not resolve the API server for context $(KUBE_CTX)"; exit 1; }; \
	OLD_UMASK=$$(umask); umask 077; \
	printf '%s\n' \
		"apiVersion: v1" "kind: Config" "current-context: $(KUBE_CTX)" \
		"clusters:" "  - name: $(KUBE_CTX)" "    cluster:" \
		"      server: $$SERVER" "      certificate-authority-data: $$CA_B64" \
		"contexts:" "  - name: $(KUBE_CTX)" "    context:" \
		"      cluster: $(KUBE_CTX)" "      namespace: $(NS_APP)" "      user: $(CI_DEPLOYER_SA)" \
		"users:" "  - name: $(CI_DEPLOYER_SA)" "    user:" "      token: '$$TOKEN'" \
		> "$$OUT_ABS"; \
	umask "$$OLD_UMASK"; chmod 600 "$$OUT_ABS"; \
	unset TOKEN; \
	echo "$(C_GRN)wrote$(C_RST) $$OUT_ABS (mode 600, contents not printed)"
	$(MAKE) --no-print-directory verify-ci-credential ENV=$(ENV)

# The `no` rows pass --all-namespaces so a cluster-scoped request is actually
# exercised; a namespaced one can pass on the RoleBinding alone and prove nothing.
.PHONY: verify-ci-credential
verify-ci-credential: vpn-up kube-ctx
	@test -r "$(CI_DEPLOYER_OUT)" \
		|| { echo "$(C_RED)ERROR$(C_RST): no kubeconfig at $(CI_DEPLOYER_OUT) — run 'make provision-ci ENV=$(ENV)' first"; exit 1; }
	@test -n "$(AIRBYTE_NAMESPACE)" \
		|| { echo "$(C_RED)ERROR$(C_RST): airbyte.namespace is empty in $(VALUES) — nothing to verify the airbyte supplement against"; exit 1; }
	@FAILURES=0; \
	check() { \
		WANT=$$1; DESC=$$2; shift 2; \
		GOT=$$(kubectl --kubeconfig "$(CI_DEPLOYER_OUT)" auth can-i "$$@" 2>/dev/null | head -1 | awk '{print $$1}'); \
		[ -n "$$GOT" ] || GOT="(no answer)"; \
		if [ "$$GOT" = "$$WANT" ]; then echo "$(C_GRN)OK$(C_RST)   $$DESC"; \
		else echo "$(C_RED)FAIL$(C_RST) want $$WANT got $$GOT — $$DESC"; FAILURES=$$((FAILURES + 1)); fi; \
	}; \
	check yes "create jobs in $(NS_APP) (seed stage renders a Job)"          create jobs -n $(NS_APP); \
	check yes "get pods/log in $(NS_APP) (seed + diagnostics)"               get pods/log -n $(NS_APP); \
	check yes "create secrets in $(NS_APP) (helm release storage)"          create secrets -n $(NS_APP); \
	check yes "create configmaps in $(NS_APP) (seed-stand.sh publishes the manifest)" create configmaps -n $(NS_APP); \
	check yes "update deployments in $(NS_APP) (helm upgrade, rollout)"     update deployments.apps -n $(NS_APP); \
	check yes "create Roles in $(NS_APP) (chart renders its own RBAC)"      create roles.rbac.authorization.k8s.io -n $(NS_APP); \
	check yes "create WorkflowTemplates in $(NS_APP)"                        create workflowtemplates.argoproj.io -n $(NS_APP); \
	check yes "update HTTPRoutes in $(NS_APP) (chart renders the edge routes)" update httproutes.gateway.networking.k8s.io -n $(NS_APP); \
	check yes "get PVCs in $(NS_APP) (helm reads the git-cli-proxy cache claim every upgrade)" get persistentvolumeclaims -n $(NS_APP); \
	check yes "update PVCs in $(NS_APP) (its chart/version labels change every bump)" update persistentvolumeclaims -n $(NS_APP); \
	check no  "delete PVCs in $(NS_APP) (the clone cache outlives the release)" delete persistentvolumeclaims -n $(NS_APP); \
	check yes "get Roles in $(AIRBYTE_NAMESPACE) (helm reads airbyte-auth-reader every upgrade)" get roles.rbac.authorization.k8s.io -n $(AIRBYTE_NAMESPACE); \
	check yes "update Roles in $(AIRBYTE_NAMESPACE) (its chart/version labels change every bump)" update roles.rbac.authorization.k8s.io -n $(AIRBYTE_NAMESPACE); \
	check yes "create Roles in $(AIRBYTE_NAMESPACE)"                         create roles.rbac.authorization.k8s.io -n $(AIRBYTE_NAMESPACE); \
	check yes "create RoleBindings in $(AIRBYTE_NAMESPACE) (the chart's airbyte-auth-reader)" create rolebindings.rbac.authorization.k8s.io -n $(AIRBYTE_NAMESPACE); \
	check no  "read pods in $(AIRBYTE_NAMESPACE) (the exception is RBAC-only)" get pods -n $(AIRBYTE_NAMESPACE); \
	check no  "anything, anywhere (the blanket check)"                       '*' '*' --all-namespaces; \
	check no  "list namespaces at cluster scope"                            list namespaces --all-namespaces; \
	check no  "create namespaces (hence the --create-namespace limitation)" create namespaces --all-namespaces; \
	check no  "read Secrets across all namespaces"                          get secrets --all-namespaces; \
	check no  "list nodes"                                                  list nodes --all-namespaces; \
	check no  "create ClusterRoleBindings"                                  create clusterrolebindings.rbac.authorization.k8s.io --all-namespaces; \
	check no  "read pods in kube-system"                                    get pods -n kube-system; \
	[ "$$FAILURES" -eq 0 ] || { echo "$(C_RED)ERROR$(C_RST): $$FAILURES scope assertion(s) failed — do not put this kubeconfig in a GitHub environment"; exit 1; }; \
	echo "$(C_GRN)all scope assertions hold$(C_RST)"

# Revocation is immediate: the API server re-validates the SA's UID per request.
.PHONY: revoke-ci
revoke-ci: vpn-up kube-ctx
	kubectl --context $(KUBE_CTX) delete -f $(CI_DEPLOYER_MANIFEST) --ignore-not-found
	@echo "$(C_GRN)OK$(C_RST) ci-deployer ServiceAccount, RBAC and token Secret removed from $(NS_APP)"
	@echo "$(C_YEL)NOTE$(C_RST) a GitHub environment secret may still hold the dead kubeconfig; replace or delete it"

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
# One-time install of cluster prerequisites (gateway controller,
# cert-manager, sealed-secrets-controller) for an env. Idempotent: a
# re-run is a no-op apart from helm-upgrade noise.
#
# Per-env values live at bootstrap/<ENV>/*.yaml. Defaults loaded only
# if the file exists, so an env with no overrides falls back to chart
# defaults.

.PHONY: bootstrap
bootstrap: inventory-present vpn-up kube-ctx
	$(call _run_if_enabled,bootstrap.namespaces,bootstrap-namespaces)
	$(call _run_if_enabled,bootstrap.envoyGateway,bootstrap-envoy-gateway)
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

# OCI chart — no `helm repo add/update`. Ships the Gateway API CRDs.
.PHONY: bootstrap-envoy-gateway
bootstrap-envoy-gateway: vpn-up kube-ctx
	@VALUES_ARGS=""; \
	if [ -s $(BOOTSTRAP_DIR)/envoy-gateway-values.yaml ]; then \
		VALUES_ARGS="-f $(BOOTSTRAP_DIR)/envoy-gateway-values.yaml"; \
	fi; \
	helm upgrade --install $(ENVOY_GATEWAY_RELEASE) oci://docker.io/envoyproxy/gateway-helm \
		--namespace $(ENVOY_GATEWAY_NAMESPACE) --create-namespace \
		--version $(ENVOY_GATEWAY_VERSION) $(HELM_EXTRA_FLAGS) \
		$$VALUES_ARGS \
		--wait --timeout 5m
	@if [ -f $(BOOTSTRAP_DIR)/gateway.yaml ]; then \
		echo "applying $(BOOTSTRAP_DIR)/gateway.yaml"; \
		kubectl --context $(KUBE_CTX) apply -f $(BOOTSTRAP_DIR)/gateway.yaml; \
	fi

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
		--set config.enableGatewayAPI=$(CERT_MANAGER_GATEWAY_SHIM) \
		$$VALUES_ARGS \
		--wait --timeout 5m
	@kubectl --context $(KUBE_CTX) -n $(CERT_MANAGER_NAMESPACE) \
		rollout status deploy/$(CERT_MANAGER_RELEASE)-webhook --timeout=60s
	@# INVARIANT: bootstrap returns only once every issuer this file defines is
	@# Ready — a CA issuer is not usable until its root Certificate is minted,
	@# and a release whose leaf references one too early stalls on a missing
	@# Secret until helm's atomic timeout instead of failing here by name.
	@if [ -f $(BOOTSTRAP_DIR)/selfsigned-issuer.yaml ]; then \
		echo "applying $(BOOTSTRAP_DIR)/selfsigned-issuer.yaml"; \
		kubectl --context $(KUBE_CTX) apply -f $(BOOTSTRAP_DIR)/selfsigned-issuer.yaml; \
		for issuer in $$(yq -N -r 'select(.kind == "ClusterIssuer") | .metadata.name' $(BOOTSTRAP_DIR)/selfsigned-issuer.yaml); do \
			kubectl --context $(KUBE_CTX) wait clusterissuer/$$issuer \
				--for=condition=Ready --timeout=120s; \
		done; \
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
	@echo "=== envoy-gateway ==="
	@helm --kube-context $(KUBE_CTX) -n $(ENVOY_GATEWAY_NAMESPACE) status $(ENVOY_GATEWAY_RELEASE) 2>/dev/null | head -3 || echo "(not installed)"
	@kubectl --context $(KUBE_CTX) get gatewayclass 2>/dev/null || echo "(no Gateway API CRDs)"
	@kubectl --context $(KUBE_CTX) get gateway -A 2>/dev/null || true
	@echo
	@echo "=== cert-manager ==="
	@helm --kube-context $(KUBE_CTX) -n $(CERT_MANAGER_NAMESPACE) status $(CERT_MANAGER_RELEASE) 2>/dev/null | head -3 || echo "(not installed)"
	@kubectl --context $(KUBE_CTX) get clusterissuer 2>/dev/null
	@echo
	@echo "=== sealed-secrets ==="
	@helm --kube-context $(KUBE_CTX) -n $(SEALED_SECRETS_NAMESPACE) status $(SEALED_SECRETS_RELEASE) 2>/dev/null | head -3 || echo "(not installed)"
