# Feature: Claude Team Connector — Bronze Bring-up

| Field | Value |
|---|---|
| Component | `connectors/ai/claude-team` |
| Status | MVP |
| Tracking issue | #458 |

## 1. Context

Bring up the `claude-team` declarative Airbyte source against the
customer-deployed proxy (see PRD §1.2 and DESIGN §1). Land four
streams in `bronze_claude_team.*`. (Bronze MVP scope — Silver/Gold
landed later: `class_ai_dev_usage` per INSIGHT-458 and `class_ai_overage`
/ Gold `cc_overage` per descriptor 1.3.0; see DESIGN §4.4.)

## 2. Deliverables

### 2.1 In this repo (insight)

- [x] `src/ingestion/connectors/ai/claude-team/connector.yaml`
- [x] `src/ingestion/connectors/ai/claude-team/descriptor.yaml`
- [x] `src/ingestion/connectors/ai/claude-team/README.md`
- [x] `src/ingestion/secrets/connectors/claude-team.yaml.example`
- [x] Specs (`PRD.md`, `DESIGN.md`, `FEATURE.md`)

### 2.2 In secure-enclave (separate repo)

The proxy implementation is out of scope for this PR — see
`gitlab.constr.dev/insight/secure-enclave → proxies/claude_team/`.

## 3. Acceptance tests

| # | Test | Expected |
|---|---|---|
| AT-1 | `connector.yaml validate-strict` against declarative schema | Pass |
| AT-2 | Reconcile-loop registers source from the K8s Secret without manual UUID handling | Source visible in Airbyte UI |
| AT-3 | Sync against a working proxy + a Team-plan org → `claude_team_members` has N rows where N matches the org's roster | Match |
| AT-4 | Sync against a proxy whose sessionKey lacks `billing:view` → `claude_team_overage_spend` empty, other streams populated, sync GREEN | Match |
| AT-5 | Sync against a proxy returning 401 (wrong bearer) → sync RED, no Bronze writes | Match |
| AT-6 | `claude_team_code_metrics` `metric_date` field on every row equals the cursor's day-stepped value | Match |
| AT-7 | Re-run after a one-day sync → cursor advances, no rows re-emitted for already-synced days | Match |
| AT-8 | Customer rotates sessionKey on the proxy via `POST /admin/session-key`; next Insight sync → GREEN with no Insight-side change | Match |

## 4. Out of scope

- Proxy code, Dockerfile, deployment docs → `secure-enclave` repo.
- Silver / Gold models.
- Real-time / streaming sync.
- Multi-org per single connector instance.
- Automated CI for proxy image build (lives in secure-enclave).

## 5. Risks

- **Schema drift on claude.ai's side** — they may rename fields or
  paginate where we don't expect. Mitigation: `autoImportSchema: true`
  + `additionalProperties: true` keep us tolerant; sync would fail RED
  on truly breaking changes and a follow-up PR adjusts the YAML.
- **Cookie TTL unknown** — claude.ai does not publish sessionKey
  expiry. Customer ops needs to monitor sync status and rotate when
  the proxy starts returning 503.
- **Bearer-token rotation coordination** — must be done atomically
  across the customer's container env and the Insight K8s Secret.
  Documented in the proxy README.

## 6. Dependencies

- Airbyte declarative-source schema version `7.0.4` (pinned at top of
  `connector.yaml`).
- Reconcile-loop (`src/ingestion/reconcile-connectors/`) — manages
  Airbyte source/destination/connection lifecycle from descriptors.
- ClickHouse Bronze layer — destination created by reconcile with
  namespace `bronze_claude_team`.
