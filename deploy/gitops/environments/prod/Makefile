# prod (InsightCfabric stand) — environment-scoped deploy targets.
#
# Mirrors the insight-gitops reference flow (inventory-driven targets, flat
# per-env values overlays, concrete manifests, CONFIRM-gated deploy); this
# env-scoped Makefile exists because the stand's topology — operator-managed
# datastores, one namespace per service — cannot be driven by the top-level
# Makefile's chart-based bootstrap-*/system-* targets. Same target vocabulary,
# operator implementations. Layer glossary (docs/components/deployment/specs/
# PRD.md): L0 bootstrap, L2 system, L3 app; there is no L1.
#
#   make kube-ctx           verify kubectl points at THIS stand (prerequisite of EVERY target)
#   make bootstrap          L0: storage class, Envoy Gateway (+ shared Gateway), cert-manager (+ CA chain), sealed-secrets controller
#   make secrets            apply sealed-secrets/** and wait for the controller to unseal each
#   make system             L2: clickhouse, mariadb, redis, redpanda, airbyte, argo — or per-service system-<svc>
#   make deploy CONFIRM=yes-deploy-prod
#                           L3: realm ConfigMap, argo RBAC, umbrella helm upgrade
#   make status             umbrella rollout health
#   make system-status      what's installed across the service namespaces
#   make rollback           helm rollback of the umbrella release
#
# Everything cluster-specific comes from inventory.yaml (context name, API
# server, edge host/IP, chart pin) or from the concrete committed manifests —
# there is no placeholder rendering. Component versions/namespaces/timeouts
# are pinned below with ?=; bump deliberately, override per-call when testing.
#
# SECRETS are gitops-managed: every credential is a committed SealedSecret
# under sealed-secrets/<namespace>/, applied by `make secrets` and decrypted
# only by this cluster's controller. This Makefile never generates or
# composes cleartext; system/deploy targets presence-check the unsealed
# Secrets and fail loudly and early rather than timing out twenty minutes
# later. To rotate a value, seal it with kubeseal against the controller
# (`kubeseal --controller-name <release> --controller-namespace kube-system`)
# and commit the updated manifest. Sole exception: the Airbyte-internal
# postgres password is chart-owned — system-airbyte reuses the one in the
# existing release Secret and only mints a fresh one on first install
# (passing nothing would silently fall back to the chart default).

SHELL := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c

INVENTORY     := inventory.yaml
BOOTSTRAP_DIR := ../../bootstrap/prod

# ── Inventory-driven values (CLI flags and shell env override via ?=) ────────
KUBE_CTX  ?= $(shell yq -r '.kubeContext // ""' $(INVENTORY) 2>/dev/null)
NS_APP    ?= $(shell yq -r '.namespaces.services // "insight"' $(INVENTORY) 2>/dev/null)
RELEASE   ?= $(shell yq -r '.release // "insight"' $(INVENTORY) 2>/dev/null)

# Raw cluster IPs are NOT committed — they live in secrets.local.env (gitignored),
# sourced here so they never enter the repository or the yq-visible inventory.
# Missing → empty → the guards below fail with an explicit message. CLI/env
# override still wins via ?=. See secrets.local.env.template.
LOCAL_ENV  := secrets.local.env
APISERVER  ?= $(shell [ -f $(LOCAL_ENV) ] && . ./$(LOCAL_ENV) 2>/dev/null; printf '%s' "$$APISERVER")
EDGE_LB_IP ?= $(shell [ -f $(LOCAL_ENV) ] && . ./$(LOCAL_ENV) 2>/dev/null; printf '%s' "$$EDGE_LB_IP")

# Chart version resolution — same order as the top-level Makefile:
# CLI override, then the inventory `chartVersion:` pin, then the repo-global
# .insight-version trunk pointer.
ENV_CHART_PIN := $(shell yq -r '.chartVersion // ""' $(INVENTORY) 2>/dev/null)
ifneq ($(strip $(ENV_CHART_PIN)),)
INSIGHT_VERSION ?= $(ENV_CHART_PIN)
else
INSIGHT_VERSION ?= $(shell cat ../../.insight-version 2>/dev/null)
endif

# ── Component pins (bump deliberately, same discipline as the reference) ─────
ENVOY_GATEWAY_CHART         ?= oci://docker.io/envoyproxy/gateway-helm
ENVOY_GATEWAY_VERSION       ?= v1.8.3
ENVOY_GATEWAY_NAMESPACE     ?= envoy-gateway-system
ENVOY_GATEWAY_RELEASE       ?= eg
ENVOY_GATEWAY_CLASS         ?= envoy
ENVOY_GATEWAY_NAME          ?= insight
CERT_MANAGER_VERSION        ?= v1.21.1
CERT_MANAGER_NAMESPACE      ?= cert-manager
CERT_MANAGER_CA_ISSUER      ?= insight-ca
CLICKHOUSE_OPERATOR_VERSION ?= 0.27.1
CLICKHOUSE_NAMESPACE        ?= clickhouse
CLICKHOUSE_REPLICAS         ?= 1
KEEPER_REPLICAS             ?= 1
MARIADB_OPERATOR_VERSION    ?= 26.6.0
MARIADB_NAMESPACE           ?= mariadb
REDIS_OPERATOR_VERSION      ?= 0.25.0
REDIS_CHART_VERSION         ?= 0.17.0
REDIS_NAMESPACE             ?= redis
REDPANDA_VERSION            ?= 26.1.9
REDPANDA_NAMESPACE          ?= redpanda
AIRBYTE_VERSION             ?= 1.8.5
AIRBYTE_NAMESPACE           ?= airbyte
ARGO_VERSION                ?= 1.0.22
ARGO_NAMESPACE              ?= argo
INSIGHT_CHART               ?= oci://ghcr.io/constructorfabric/charts/insight

# ── Timeouts and auxiliary pins ───────────────────────────────────────────────
TIMEOUT               ?= 10m
ENVOY_GATEWAY_TIMEOUT ?= 10m
CERT_MANAGER_TIMEOUT  ?= 10m
MARIADB_WAIT_TIMEOUT  ?= 15m
REDIS_TIMEOUT         ?= 15m
REDPANDA_TIMEOUT      ?= 15m
AIRBYTE_TIMEOUT       ?= 20m
INSIGHT_TIMEOUT       ?= 20m
AIRBYTE_CURL_IMAGE    ?= curlimages/curl:8.11.0
# Cosmetic (they name the Airbyte workspace owner, not a mailbox) but the
# setup call must happen: with auth enabled a fresh instance sits
# half-initialised in a wizard until email + organisation are posted.
AIRBYTE_SETUP_EMAIL   ?= admin@example.com
AIRBYTE_SETUP_ORG     ?= Insight

# ── Datastore-namespace credential Secret names ──────────────────────────────
# Presence-checked before applying CRs (the CRs reference them; a missing one
# otherwise surfaces as an opaque wait timeout). Sealed manifests:
# sealed-secrets/<namespace>/.
CLICKHOUSE_DEFAULT_SECRET ?= clickhouse-default-credentials
CLICKHOUSE_INSIGHT_SECRET ?= clickhouse-insight-credentials
MARIADB_ROOT_SECRET       ?= mariadb-root
MARIADB_INSIGHT_SECRET    ?= mariadb-insight-credentials
REDIS_AUTH_SECRET         ?= redis-auth

# Helm 4 applies server-side and needs the mode NAMED, not inherited: on 4.2+
# `--server-side` defaults to "whatever the previous revision used", and
# --force-conflicts alone against a client-side release is rejected outright.
# Helm 3 gets no flags.
HELM_UPGRADE_HELP := $(shell helm upgrade --help 2>/dev/null)
HELM_APPLY_FLAGS := $(if $(findstring --server-side,$(HELM_UPGRADE_HELP)),--server-side=true --force-conflicts,$(if $(findstring --force-conflicts,$(HELM_UPGRADE_HELP)),--force-conflicts,))

# ── Sealed-secrets pins ──────────────────────────────────────────────────────
# Everything under sealed-secrets/ is sealed against THIS cluster's controller
# key and decrypts only here. Re-seal everything if the key rotates or the
# cluster is rebuilt.
# ≥2.18.2 required: earlier charts ship a controller below 0.36.0, the fix
# for GHSA-465p-v42x-3fmj (rotate-endpoint sealing-scope widening).
SEALED_SECRETS_VERSION   ?= 2.19.1
SEALED_SECRETS_NAMESPACE ?= kube-system
SEALED_SECRETS_RELEASE   ?= sealed-secrets-controller

.PHONY: kube-ctx confirm bootstrap system deploy deploy-app deploy-insight \
        bootstrap-storage bootstrap-envoy-gateway bootstrap-cert-manager bootstrap-sealed-secrets \
        system-clickhouse system-mariadb system-redis system-redpanda system-airbyte system-argo \
        keycloak-broker-realms argo-rbac status system-status rollback secrets

# Both stand kubeconfigs ship their context named `default` — rename it to
# `insight-prod` once per machine (repo README prerequisite #4):
#   kubectl config rename-context default insight-prod
# The context name alone identifies nothing, so the API server URL from the
# inventory is the real wrong-cluster protection. Every mutating target
# depends on this — and every kubectl/helm invocation below pins
# --context/--kube-context to $(KUBE_CTX), so a context switch in another
# shell between this check and a mutation cannot redirect the mutation.
kube-ctx:
	@[ -n "$(APISERVER)" ] || { \
	  echo "ERROR: APISERVER is empty — copy secrets.local.env.template to"; \
	  echo "       secrets.local.env and fill in the stand's API server URL."; \
	  exit 1; }
	@kubectl config get-contexts -o name 2>/dev/null | grep -qx '$(KUBE_CTX)' || { \
	  echo "ERROR: kube-context '$(KUBE_CTX)' not found in the active kubeconfig"; \
	  echo "       (first time: kubectl config rename-context default $(KUBE_CTX))"; \
	  exit 1; }; \
	live=$$(kubectl config view --minify --context '$(KUBE_CTX)' -o 'jsonpath={.clusters[0].cluster.server}'); \
	if [ "$$live" != "$(APISERVER)" ]; then \
	  echo "ERROR: context '$(KUBE_CTX)' points at a different API server than this"; \
	  echo "       environment expects — check KUBECONFIG (addresses not printed)."; \
	  exit 1; \
	fi; echo "kube-ctx OK: $(KUBE_CTX) (API server matches)"

# Protected env (inventory.protected): deploy refuses without the token.
# Fails CLOSED: only an explicit `protected: false` skips the gate — a missing
# yq, an unreadable inventory, or a malformed value all count as protected.
confirm:
	@prot=$$(yq -r '.protected // "unreadable"' $(INVENTORY) 2>/dev/null) || prot=unreadable; \
	if [ "$$prot" != "false" ]; then \
	  if [ "$(CONFIRM)" != "yes-deploy-prod" ]; then \
	    echo "ERROR: deploy to 'prod' requires CONFIRM=yes-deploy-prod (inventory.protected=$$prot; anything but 'false' protects)"; \
	    exit 1; \
	  fi; \
	fi

# ── L0: bootstrap — cluster foundation ───────────────────────────────────────
# Order is load-bearing: the Envoy Gateway chart installs the Gateway API CRDs
# that cert-manager (enableGatewayAPI) crashes without, and the https listener
# cannot program until cert-manager has minted the origin certificate.
bootstrap: kube-ctx bootstrap-storage bootstrap-envoy-gateway bootstrap-cert-manager bootstrap-sealed-secrets

bootstrap-storage: kube-ctx
	@# reclaimPolicy is immutable, so changing it means deleting and recreating
	@# the class. Bound PVCs and their PVs survive that — a PV carries its own
	@# copy of the policy — but a PVC still waiting to be provisioned does not.
	@want=$$(yq -r 'select(.metadata.name == "cinder") | .reclaimPolicy' $(BOOTSTRAP_DIR)/cinder-storageclass.yaml); \
	have=$$(kubectl --context $(KUBE_CTX) get storageclass cinder -o jsonpath='{.reclaimPolicy}' 2>/dev/null || true); \
	if [ -n "$$have" ] && [ "$$have" != "$$want" ]; then \
	  unbound=$$(kubectl --context $(KUBE_CTX) get pvc -A -o jsonpath='{range .items[?(@.status.phase!="Bound")]}{.metadata.namespace}/{.metadata.name} {end}'); \
	  [ -z "$$unbound" ] || { echo "ERROR: recreating the cinder class would strand unbound PVCs: $$unbound"; exit 1; }; \
	  echo "cinder reclaimPolicy $$have -> $$want; recreating the class"; \
	  kubectl --context $(KUBE_CTX) delete storageclass cinder; \
	fi
	kubectl --context $(KUBE_CTX) apply -f $(BOOTSTRAP_DIR)/cinder-storageclass.yaml
	@# A PV keeps the policy it was provisioned under, so the class change does
	@# not reach volumes that already exist. On a PV the field is mutable.
	@for pv in $$(kubectl --context $(KUBE_CTX) get pv -o go-template='{{range .items}}{{if and (eq .spec.storageClassName "cinder") (ne .spec.persistentVolumeReclaimPolicy "Retain")}}{{.metadata.name}}{{"\n"}}{{end}}{{end}}'); do \
	  kubectl --context $(KUBE_CTX) patch pv $$pv -p '{"spec":{"persistentVolumeReclaimPolicy":"Retain"}}'; \
	done

bootstrap-envoy-gateway: kube-ctx
	helm upgrade --kube-context $(KUBE_CTX) --install $(ENVOY_GATEWAY_RELEASE) $(ENVOY_GATEWAY_CHART) \
	  --version $(ENVOY_GATEWAY_VERSION) \
	  --namespace $(ENVOY_GATEWAY_NAMESPACE) --create-namespace \
	  --wait --timeout $(ENVOY_GATEWAY_TIMEOUT) $(HELM_APPLY_FLAGS)
	@# The floating IP is not committed (secrets.local.env). Substitute it into
	@# the EnvoyProxy at apply time; EDGE_LB_IP=auto (or empty) drops the
	@# loadBalancerIP line so a fresh stand gets a cloud-allocated address.
	@[ -n "$(EDGE_LB_IP)" ] || { echo "ERROR: EDGE_LB_IP is empty — set it in secrets.local.env (or 'auto')"; exit 1; }
	@if [ "$(EDGE_LB_IP)" = auto ]; then \
	  sed '/loadBalancerIP: __EDGE_LB_IP__/d' $(BOOTSTRAP_DIR)/envoyproxy.yaml; \
	else \
	  sed 's|__EDGE_LB_IP__|$(EDGE_LB_IP)|' $(BOOTSTRAP_DIR)/envoyproxy.yaml; \
	fi | kubectl --context $(KUBE_CTX) apply -f -
	kubectl --context $(KUBE_CTX) apply -f $(BOOTSTRAP_DIR)/gateway.yaml
	kubectl --context $(KUBE_CTX) wait --for=condition=Accepted gatewayclass/$(ENVOY_GATEWAY_CLASS) --timeout=120s
	@# Accepted AND addressed: the floating IP is the invariant everything
	@# downstream (DNS, Cloudflare) points at. A still-attached-elsewhere IP
	@# shows up here as a timeout — delete the Service that holds it. Addresses
	@# are redacted to the last octet so they stay out of the logs.
	@deadline=$$((SECONDS + 600)); while :; do \
	  addr=$$(kubectl --context $(KUBE_CTX) -n $(ENVOY_GATEWAY_NAMESPACE) get gateway/$(ENVOY_GATEWAY_NAME) \
	    -o 'jsonpath={.status.addresses[0].value}' 2>/dev/null || true); \
	  if [ -n "$$addr" ]; then \
	    red=$$(printf '%s' "$$addr" | sed 's/[0-9]*\.[0-9]*\.[0-9]*\./x.x.x./'); \
	    echo "Gateway addressed (…$$red)"; \
	    { [ "$$addr" = "$(EDGE_LB_IP)" ] || [ "$(EDGE_LB_IP)" = auto ]; } || \
	      echo "WARNING: address differs from the pinned EDGE_LB_IP"; break; \
	  fi; \
	  [ $$SECONDS -lt $$deadline ] || { echo "ERROR: Gateway never got an address —"; \
	    echo "  is the floating IP still attached to another Service?"; exit 1; }; \
	  sleep 5; done

bootstrap-cert-manager: kube-ctx
	helm repo add jetstack https://charts.jetstack.io --force-update
	helm upgrade --kube-context $(KUBE_CTX) --install cert-manager jetstack/cert-manager \
	  --version $(CERT_MANAGER_VERSION) \
	  --namespace $(CERT_MANAGER_NAMESPACE) --create-namespace \
	  --set crds.enabled=true --set config.enableGatewayAPI=true \
	  --wait --timeout $(CERT_MANAGER_TIMEOUT) $(HELM_APPLY_FLAGS)
	@# Retry the issuer apply: the validating webhook races the Deployment
	@# becoming Ready on a fresh install and rejects early applies.
	@for i in $$(seq 1 30); do \
	  kubectl --context $(KUBE_CTX) apply -f $(BOOTSTRAP_DIR)/cluster-issuers.yaml && break; \
	  [ $$i -lt 30 ] || { echo "ERROR: cluster-issuers apply kept failing (webhook down?)"; exit 1; }; \
	  echo "  webhook not ready, retrying ($$i/30)"; sleep 5; done
	kubectl --context $(KUBE_CTX) wait --for=condition=Ready clusterissuer/insight-selfsigned --timeout=120s
	kubectl --context $(KUBE_CTX) -n $(CERT_MANAGER_NAMESPACE) wait --for=condition=Ready certificate/insight-ca --timeout=300s
	kubectl --context $(KUBE_CTX) wait --for=condition=Ready clusterissuer/$(CERT_MANAGER_CA_ISSUER) --timeout=120s
	@# The Gateway's origin certificate (gateway-shim mints it from the
	@# listener annotation) — only expected once the Gateway exists.
	@if kubectl --context $(KUBE_CTX) -n $(ENVOY_GATEWAY_NAMESPACE) get gateway/$(ENVOY_GATEWAY_NAME) >/dev/null 2>&1; then \
	  kubectl --context $(KUBE_CTX) -n $(ENVOY_GATEWAY_NAMESPACE) wait --for=condition=Ready \
	    certificate/insight-origin-tls --timeout=300s; fi

# ── L2: system — datastores and infrastructure services ──────────────────────
system: kube-ctx system-clickhouse system-mariadb system-redis system-redpanda system-airbyte system-argo

system-clickhouse: kube-ctx
	helm repo add clickhouse-operator https://helm.altinity.com --force-update
	helm upgrade --kube-context $(KUBE_CTX) --install clickhouse-operator clickhouse-operator/altinity-clickhouse-operator \
	  --version $(CLICKHOUSE_OPERATOR_VERSION) \
	  --namespace clickhouse-operator --create-namespace \
	  --set "configs.files.config\.yaml.watch.namespaces.include[0]=$(CLICKHOUSE_NAMESPACE)" \
	  --wait --timeout $(TIMEOUT) $(HELM_APPLY_FLAGS)
	kubectl --context $(KUBE_CTX) wait --for=condition=Established crd/clickhouseinstallations.clickhouse.altinity.com --timeout=120s
	kubectl --context $(KUBE_CTX) wait --for=condition=Established crd/clickhousekeeperinstallations.clickhouse-keeper.altinity.com --timeout=120s
	kubectl --context $(KUBE_CTX) create namespace $(CLICKHOUSE_NAMESPACE) --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -
	@# The CHI references these user Secrets; without them ClickHouse can
	@# never start and the failure is an opaque pod crash-loop much later.
	@for s in $(CLICKHOUSE_DEFAULT_SECRET) $(CLICKHOUSE_INSIGHT_SECRET); do \
	  kubectl --context $(KUBE_CTX) -n $(CLICKHOUSE_NAMESPACE) get secret $$s >/dev/null 2>&1 || \
	    { echo "ERROR: Secret $(CLICKHOUSE_NAMESPACE)/$$s missing — run 'make secrets'"; exit 1; }; done
	kubectl --context $(KUBE_CTX) apply -f manifests/clickhouse/keeper.yaml
	kubectl --context $(KUBE_CTX) apply -f manifests/clickhouse/clickhouse.yaml
	@echo "waiting for $(KEEPER_REPLICAS) keeper + $(CLICKHOUSE_REPLICAS) clickhouse pod(s)"
	@deadline=$$((SECONDS + 900)); while :; do \
	  ready=$$(kubectl --context $(KUBE_CTX) -n $(CLICKHOUSE_NAMESPACE) get pods \
	    -l 'clickhouse.altinity.com/chi=clickhouse' \
	    -o 'jsonpath={range .items[*]}{.status.conditions[?(@.type=="Ready")].status}{"\n"}{end}' 2>/dev/null | grep -c True || true); \
	  [ "$$ready" -ge "$(CLICKHOUSE_REPLICAS)" ] && { echo "clickhouse ready ($$ready pod(s))"; break; }; \
	  [ $$SECONDS -lt $$deadline ] || { echo "ERROR: clickhouse pods never became Ready"; exit 1; }; \
	  sleep 10; done

system-mariadb: kube-ctx
	helm upgrade --kube-context $(KUBE_CTX) --install mariadb-operator-crds oci://ghcr.io/mariadb-operator/charts/mariadb-operator-crds \
	  --version $(MARIADB_OPERATOR_VERSION) \
	  --namespace mariadb-operator --create-namespace \
	  --wait --timeout $(TIMEOUT) $(HELM_APPLY_FLAGS)
	helm upgrade --kube-context $(KUBE_CTX) --install mariadb-operator oci://ghcr.io/mariadb-operator/charts/mariadb-operator \
	  --version $(MARIADB_OPERATOR_VERSION) \
	  --namespace mariadb-operator --create-namespace \
	  --wait --timeout $(TIMEOUT) $(HELM_APPLY_FLAGS)
	@for crd in mariadbs.k8s.mariadb.com databases.k8s.mariadb.com users.k8s.mariadb.com grants.k8s.mariadb.com; do \
	  kubectl --context $(KUBE_CTX) wait --for=condition=Established crd/$$crd --timeout=120s; done
	kubectl --context $(KUBE_CTX) create namespace $(MARIADB_NAMESPACE) --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -
	@# The CR references the root Secret at spec level; the insight secret is
	@# what the User/Grant CRs and every consumer read.
	@for s in $(MARIADB_ROOT_SECRET) $(MARIADB_INSIGHT_SECRET); do \
	  kubectl --context $(KUBE_CTX) -n $(MARIADB_NAMESPACE) get secret $$s >/dev/null 2>&1 || \
	    { echo "ERROR: Secret $(MARIADB_NAMESPACE)/$$s missing — run 'make secrets'"; exit 1; }; done
	kubectl --context $(KUBE_CTX) apply -f manifests/mariadb/mariadb.yaml
	kubectl --context $(KUBE_CTX) apply -f manifests/mariadb/insight.yaml
	kubectl --context $(KUBE_CTX) -n $(MARIADB_NAMESPACE) wait --for=condition=Ready mariadb/mariadb --timeout=$(MARIADB_WAIT_TIMEOUT)
	kubectl --context $(KUBE_CTX) -n $(MARIADB_NAMESPACE) wait --for=condition=Ready database/insight --timeout=300s

system-redis: kube-ctx
	helm repo add ot-helm https://ot-container-kit.github.io/helm-charts --force-update
	helm upgrade --kube-context $(KUBE_CTX) --install redis-operator ot-helm/redis-operator \
	  --version $(REDIS_OPERATOR_VERSION) \
	  --namespace redis-operator --create-namespace \
	  --values redis-operator-values.yaml \
	  --wait --timeout $(TIMEOUT) $(HELM_APPLY_FLAGS)
	kubectl --context $(KUBE_CTX) create namespace $(REDIS_NAMESPACE) --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -
	@kubectl --context $(KUBE_CTX) -n $(REDIS_NAMESPACE) get secret $(REDIS_AUTH_SECRET) >/dev/null 2>&1 || \
	  { echo "ERROR: Secret $(REDIS_NAMESPACE)/$(REDIS_AUTH_SECRET) missing — run 'make secrets'"; exit 1; }
	helm upgrade --kube-context $(KUBE_CTX) --install redis ot-helm/redis-replication \
	  --version $(REDIS_CHART_VERSION) \
	  --namespace $(REDIS_NAMESPACE) \
	  --values redis-values.yaml \
	  --wait --timeout $(REDIS_TIMEOUT) $(HELM_APPLY_FLAGS)
	@# helm --wait proves the StatefulSet rolled out; the authenticator needs
	@# a resolvable master. The operator labels the elected leader.
	@deadline=$$((SECONDS + 300)); while :; do \
	  masters=$$( { kubectl --context $(KUBE_CTX) -n $(REDIS_NAMESPACE) get pods -l redis-role=master -o name 2>/dev/null || true; } | wc -l | tr -d ' '); \
	  [ "$$masters" = 1 ] && { echo "redis master elected"; break; }; \
	  [ $$SECONDS -lt $$deadline ] || { echo "ERROR: no single redis master elected"; exit 1; }; \
	  sleep 5; done

system-redpanda: kube-ctx
	helm repo add redpanda https://charts.redpanda.com --force-update
	helm upgrade --kube-context $(KUBE_CTX) --install redpanda redpanda/redpanda \
	  --version $(REDPANDA_VERSION) \
	  --namespace $(REDPANDA_NAMESPACE) --create-namespace \
	  --values redpanda-values.yaml \
	  --wait --timeout $(REDPANDA_TIMEOUT) $(HELM_APPLY_FLAGS)

system-airbyte: kube-ctx
	kubectl --context $(KUBE_CTX) create namespace $(AIRBYTE_NAMESPACE) --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -
	kubectl --context $(KUBE_CTX) apply -f manifests/airbyte/pvcs.yaml
	@# Root-owned chown Jobs for the Cinder fsGroup gap. Delete leftovers
	@# first (a completed Job's template is immutable), then wait: helm must
	@# not race the ownership fix.
	kubectl --context $(KUBE_CTX) -n $(AIRBYTE_NAMESPACE) delete job -l app.kubernetes.io/name=airbyte-storage-init --ignore-not-found
	kubectl --context $(KUBE_CTX) apply -f manifests/airbyte/storage-init.yaml
	kubectl --context $(KUBE_CTX) -n $(AIRBYTE_NAMESPACE) wait --for=condition=Complete job -l app.kubernetes.io/name=airbyte-storage-init --timeout=$(AIRBYTE_TIMEOUT)
	kubectl --context $(KUBE_CTX) apply -f manifests/airbyte/rbac.yaml
	helm repo add airbyte https://airbytehq.github.io/helm-charts --force-update
	@# Postgres password: reuse the chart-generated one when the release
	@# already exists; ONLY an explicit NotFound mints a new one — any other
	@# lookup failure (RBAC, timeout, transport) aborts, because silently
	@# minting a fresh password against an existing database bricks it.
	@# Passing nothing would fall back to the chart default (literal 'airbyte').
	@if out=$$(kubectl --context $(KUBE_CTX) -n $(AIRBYTE_NAMESPACE) get secret airbyte-airbyte-secrets \
	  -o 'jsonpath={.data.DATABASE_PASSWORD}' 2>&1); then \
	  pw=$$(printf '%s' "$$out" | base64 --decode); \
	  [ -n "$$pw" ] || { echo "ERROR: airbyte-airbyte-secrets exists but DATABASE_PASSWORD is empty"; exit 1; }; \
	elif printf '%s' "$$out" | grep -q 'NotFound'; then \
	  pw=$$(openssl rand -hex 24); \
	else \
	  echo "ERROR: could not read airbyte-airbyte-secrets (not a NotFound — refusing to mint a new password):"; \
	  echo "       $$out"; exit 1; \
	fi; \
	helm upgrade --kube-context $(KUBE_CTX) --install airbyte airbyte/airbyte \
	  --version $(AIRBYTE_VERSION) \
	  --namespace $(AIRBYTE_NAMESPACE) \
	  --values airbyte-values.yaml \
	  --set-string "postgresql.postgresqlPassword=$$pw" \
	  --wait --wait-for-jobs --timeout $(AIRBYTE_TIMEOUT) $(HELM_APPLY_FLAGS)
	@# One-shot instance setup: with auth enabled a fresh instance sits in a
	@# wizard until email+org are posted. Idempotent — the endpoint answers
	@# 2xx on an already-configured instance too, so it is replayed every run,
	@# from inside the cluster so it exercises the Service DNS name. Any
	@# non-2xx fails the pod, which fails the wait below — no masked errors.
	@kubectl --context $(KUBE_CTX) -n $(AIRBYTE_NAMESPACE) delete pod airbyte-setup-oneshot --ignore-not-found >/dev/null 2>&1; \
	kubectl --context $(KUBE_CTX) -n $(AIRBYTE_NAMESPACE) run airbyte-setup-oneshot --restart=Never --image=$(AIRBYTE_CURL_IMAGE) \
	  --command -- sh -c 'B=http://airbyte-airbyte-server-svc:8001; \
	    curl -sf -X POST "$$B/api/v1/instance_configuration/setup" \
	      -H "Content-Type: application/json" \
	      -d "{\"email\":\"$(AIRBYTE_SETUP_EMAIL)\",\"organizationName\":\"$(AIRBYTE_SETUP_ORG)\",\"initialSetupComplete\":true,\"anonymousDataCollection\":false}"'
	kubectl --context $(KUBE_CTX) -n $(AIRBYTE_NAMESPACE) wait --for=jsonpath='{.status.phase}'=Succeeded pod/airbyte-setup-oneshot --timeout=300s
	kubectl --context $(KUBE_CTX) -n $(AIRBYTE_NAMESPACE) delete pod airbyte-setup-oneshot --ignore-not-found

system-argo: kube-ctx
	helm repo add argo https://argoproj.github.io/argo-helm --force-update
	helm upgrade --kube-context $(KUBE_CTX) --install argo-workflows argo/argo-workflows \
	  --version $(ARGO_VERSION) \
	  --namespace $(ARGO_NAMESPACE) --create-namespace \
	  --values argo-workflows-values.yaml \
	  --wait --timeout $(TIMEOUT) $(HELM_APPLY_FLAGS)

# ── L3: deploy — the Insight release ─────────────────────────────────────────
deploy: deploy-app
deploy-app: deploy-insight

# Realm content is configuration-as-code (ADR-0003): the committed YAML is
# packed VERBATIM into the ConfigMap the umbrella's keycloak-config-cli hook
# Job re-applies on every deploy — drift reverts on sync. Same shape as the
# top-level keycloak-broker-realms target.
keycloak-broker-realms: kube-ctx
	kubectl --context $(KUBE_CTX) create namespace $(NS_APP) --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -
	kubectl --context $(KUBE_CTX) -n $(NS_APP) create configmap $(RELEASE)-keycloak-config-realms \
	  --from-file=keycloak/realms \
	  --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -

argo-rbac: kube-ctx
	kubectl --context $(KUBE_CTX) create namespace $(NS_APP) --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -
	kubectl --context $(KUBE_CTX) apply -f manifests/argo-rbac.yaml

deploy-insight: kube-ctx confirm keycloak-broker-realms argo-rbac
	@for s in insight-db-creds insight-oidc insight-keycloak-admin insight-keycloak-config github-oauth insight-authenticator-signing-keys; do \
	  kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret $$s >/dev/null 2>&1 || \
	    { echo "ERROR: Secret $(NS_APP)/$$s missing — run 'make secrets'"; exit 1; }; done
	@# Read the client secret first and refuse to deploy with an empty one —
	@# a failed substitution inline in --set-string would deploy silently.
	@# Follow-up (chart change): consume the Secret via secretKeyRef so the
	@# value never transits Helm values / the process argument list at all.
	@cs=$$(kubectl --context $(KUBE_CTX) -n $(NS_APP) get secret insight-oidc -o 'jsonpath={.data.client-secret}' | base64 --decode); \
	[ -n "$$cs" ] || { echo "ERROR: insight-oidc exists but client-secret is empty"; exit 1; }; \
	helm upgrade --kube-context $(KUBE_CTX) --install $(RELEASE) $(INSIGHT_CHART) \
	  --version $(INSIGHT_VERSION) \
	  --namespace $(NS_APP) --create-namespace \
	  --values values.yaml \
	  --set-string authenticator.oidc.clientSecret="$$cs" \
	  --wait --timeout $(INSIGHT_TIMEOUT) $(HELM_APPLY_FLAGS)
	@# The composed config Secrets are read via envFrom at container start and
	@# the chart's checksum annotation does not cover them: without this
	@# restart, rotated credentials/hosts are silently not picked up.
	@for d in authenticator analytics; do \
	  kubectl --context $(KUBE_CTX) -n $(NS_APP) rollout restart deploy/$(RELEASE)-$$d; \
	  kubectl --context $(KUBE_CTX) -n $(NS_APP) rollout status deploy/$(RELEASE)-$$d --timeout=$(INSIGHT_TIMEOUT); done

# ── Sealed secrets ────────────────────────────────────────────────────────────
bootstrap-sealed-secrets: kube-ctx
	helm repo add sealed-secrets https://bitnami.github.io/sealed-secrets --force-update
	helm upgrade --kube-context $(KUBE_CTX) --install $(SEALED_SECRETS_RELEASE) sealed-secrets/sealed-secrets \
	  --version $(SEALED_SECRETS_VERSION) \
	  --namespace $(SEALED_SECRETS_NAMESPACE) \
	  --set fullnameOverride=$(SEALED_SECRETS_RELEASE) \
	  --wait --timeout $(TIMEOUT) $(HELM_APPLY_FLAGS)

# Apply every sealed manifest and wait for the controller to unseal each into
# a live Secret. Run AFTER bootstrap (the controller must exist) and BEFORE
# system/deploy (their presence checks expect the Secrets). Namespaces are
# created here because sealed manifests reference them before the system
# targets would create them.
secrets: kube-ctx
	@for d in sealed-secrets/*/; do \
	  ns=$$(basename $$d); \
	  kubectl --context $(KUBE_CTX) create namespace $$ns --dry-run=client -o yaml | kubectl --context $(KUBE_CTX) apply -f -; \
	  kubectl --context $(KUBE_CTX) apply -f $$d; done
	@echo "waiting for the controller to unseal"; \
	for f in sealed-secrets/*/*-sealedsecret.yaml; do \
	  ns=$$(basename $$(dirname $$f)); name=$$(basename $$f -sealedsecret.yaml); \
	  kubectl --context $(KUBE_CTX) -n $$ns wait --for=condition=Synced sealedsecret/$$name --timeout=120s; \
	  kubectl --context $(KUBE_CTX) -n $$ns get secret $$name >/dev/null; done; \
	echo "secrets OK: every SealedSecret is Synced and its Secret exists"

# ── Health / rollback ─────────────────────────────────────────────────────────
status: kube-ctx
	helm --kube-context $(KUBE_CTX) -n $(NS_APP) status $(RELEASE) || true
	kubectl --context $(KUBE_CTX) -n $(NS_APP) get deploy,pod

system-status: kube-ctx
	@for ns in $(CLICKHOUSE_NAMESPACE) $(MARIADB_NAMESPACE) $(REDIS_NAMESPACE) $(REDPANDA_NAMESPACE) $(AIRBYTE_NAMESPACE) $(ARGO_NAMESPACE); do \
	  echo "── $$ns"; kubectl --context $(KUBE_CTX) -n $$ns get pod 2>/dev/null || true; done

rollback: kube-ctx confirm
	helm --kube-context $(KUBE_CTX) -n $(NS_APP) rollback $(RELEASE)
