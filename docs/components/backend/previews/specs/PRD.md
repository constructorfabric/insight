---
status: proposed
date: 2026-08-27
---

# PRD -- Previews Service

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Experiment Management](#51-experiment-management)
  - [5.2 Guardrails](#52-guardrails)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [8.1 Publish a Branch Preview](#81-publish-a-branch-preview)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The previews service turns preview-environment provisioning into a self-service API. A preview experiment is one frontend build from a branch, served under `/exp/<name>` on the single shared preview host. Today an operator provisions it by hand with Helm (`deploy/preview/`); this service manages the same per-experiment Kubernetes object trio -- a `Deployment`, a `Service`, and a Gateway API `HTTPRoute` -- through a small authenticated REST API, so a non-technical user can spin up a branch's frontend and tear it down without helm, kubectl, or git.

Kubernetes is the only store: the three objects ARE the experiment, and the experiment's metadata (creator, image tag, creation time, expiry) lives in annotations on those objects. The service holds no database.

### 1.2 Background / Problem Statement

The Envoy Gateway edge made per-experiment routing a plain Kubernetes API operation: each experiment's `HTTPRoute` attaches itself to the shared Gateway over xDS, so adding or removing an experiment rewrites no central configuration and reloads no proxy. The manual chart flow works but requires cluster credentials and Helm knowledge, which limits previews to operators. This service is phase 1 of the self-service epic: the API only. Deploy wiring (chart, namespace, RBAC, gateway route) and the frontend UI follow as separate phases.

### 1.3 Goals (Business Outcomes)

- Create, list, and delete a preview experiment through an authenticated HTTP API -- no cluster access for the user.
- Keep the provisioned shape byte-compatible with the manual `deploy/preview/` chart, which remains the escape hatch and the shape reference.
- Bound the blast radius by construction: one namespace, one hardcoded image repository, a fixed pod shape, a cap on live experiments, and a TTL that removes forgotten experiments.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Experiment | One preview deployment of the frontend: the `Deployment` + `Service` + `HTTPRoute` trio named `preview-<name>`, serving under `/exp/<name>` on the shared preview host. |
| Experiment name | The URL and resource slug: a DNS-1123 label of at most 55 characters, so `preview-<name>` always fits the 63-character Kubernetes name limit. |
| Image tag | The frontend image tag an experiment pins. The repository is hardcoded to the product frontend image; only the tag varies, and only CI-published tag shapes are accepted. |
| TTL | Days until an experiment expires. A background sweep deletes expired experiments. |
| Shared Gateway | The cluster's Envoy Gateway the per-experiment `HTTPRoute` attaches to via `parentRefs`. |

## 2. Actors

### 2.1 Human Actors

#### Experiment Author

**ID**: `cpt-insightspec-previews-actor-author`

**Role**: Any authenticated user creating or removing a preview of a frontend branch build.
**Needs**: Create an experiment from a name and an image tag, see the live experiments and their URLs, delete an experiment.

### 2.2 System Actors

#### Kubernetes API Server

**ID**: `cpt-insightspec-previews-actor-kube-api`

**Role**: The only backend. Stores the experiment trio, reports readiness, and enforces RBAC (granted in the deploy-wiring phase) scoping the service to its one namespace.

#### API Gateway

**ID**: `cpt-insightspec-previews-actor-gateway`

**Role**: Fronts the service; verifies the session and forwards the signed gateway JWT the service reads its caller identity from.

## 3. Operational Concept & Environment

The service runs as a single-replica gears-rust host inside the cluster, behind the API gateway. It reaches the Kubernetes API with its in-cluster service account and operates exclusively in one configured namespace (default `insight-previews`). It is structurally inert on a production stand: experiments are a gated capability enforced at the authenticator's login-return path, so a production stand never hands out a session into `/exp/`.

### 3.1 Module-Specific Environment Constraints

- Requires a reachable Kubernetes API; the service fails its boot without one.
- Requires the shared Gateway to exist for routes to attach to; a missing Gateway surfaces as an experiment that never becomes ready, not as an API failure.
- Single replica: the TTL sweep is a plain interval task with no leader election.

## 4. Scope

### 4.1 In Scope

- The `v1` experiments API: list, create, delete.
- Typed Kubernetes object builders that replicate the manual chart's rendered shape.
- Validation of the experiment name and image tag; TTL defaulting and bounds; a cap on live experiments; the TTL sweep.
- Creator, tag, and lifetime stamped as annotations on the created objects.

### 4.2 Out of Scope

- Deploy wiring: the service's own Helm chart, the namespace, RBAC, umbrella values, and the gateway route (phase 2).
- The frontend self-service page and its capability gate (phase 3).
- Registry tag listing and TTL/cap surfacing polish (phase 4).
- Any per-experiment backend: experiments are frontend-only and share the stand's backend.

## 5. Functional Requirements

### 5.1 Experiment Management

#### Experiment Lifecycle API

- [ ] `p1` - **ID**: `cpt-insightspec-previews-fr-lifecycle-api`

The service exposes an authenticated REST API: list the live experiments with `{name, tag, url, creator, createdAt, expiresAt, status}`, create an experiment from `{name, tag}` with an optional `ttlDays`, and delete an experiment by name. Create returns the created record; deleting a non-existent experiment is a not-found error; creating a name that already exists is an already-exists error. Errors use the platform's canonical RFC 9457 envelope.

#### Kubernetes as the Only Store

- [ ] `p1` - **ID**: `cpt-insightspec-previews-fr-kube-store`

An experiment is exactly its `Deployment` + `Service` + `HTTPRoute` trio, all named `preview-<name>` and labeled with the experiment label. Metadata (creator, image tag, created-at, expires-at) is carried as annotations. Listing reads the cluster; there is no database and no second source of truth. A create that fails partway rolls back the objects it already created.

#### Chart-Equivalent Provisioned Shape

- [ ] `p1` - **ID**: `cpt-insightspec-previews-fr-chart-parity`

The built objects replicate the manual chart's render contract: one replica serving the pinned frontend image on port 8080 with health probes and pinned resources; a `ClusterIP` service on port 80 targeting it; an `HTTPRoute` attaching to the shared Gateway that matches `/exp/<name>` as a path prefix and strips it before the pod. Unit tests pin this parity against the chart's render tests.

### 5.2 Guardrails

#### Input Validation

- [ ] `p1` - **ID**: `cpt-insightspec-previews-fr-validation`

The experiment name must be a DNS-1123 label of at most 55 characters -- the same rule and failure wording as the chart. The image repository is hardcoded to the product frontend image; the request carries a tag only, accepted solely in the CI-published shapes (a `preview-` prefixed tag or a timestamped build tag). The API offers no way to set an image repository, environment variables, commands, or any other pod field.

#### Live-Count Cap and TTL

- [ ] `p1` - **ID**: `cpt-insightspec-previews-fr-cap-ttl`

A create beyond the configured cap of live experiments is refused. Every experiment carries an expiry: the requested `ttlDays` bounded by a configured maximum, or a configured default when absent. A periodic sweep deletes experiments past their expiry.

#### Creator Identity

- [ ] `p1` - **ID**: `cpt-insightspec-previews-fr-creator`

The creating caller's identity comes from the verified gateway JWT and is stamped on the experiment; requests without an identified caller are rejected as unauthenticated. The service performs no credential verification of its own.

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Bounded Cluster Access

- [ ] `p1` - **ID**: `cpt-insightspec-previews-nfr-bounded-access`

The service only ever creates, lists, and deletes objects in its one configured namespace, and only the three experiment kinds. Every Kubernetes call is timeout-bounded.

#### Offline Contract

- [ ] `p1` - **ID**: `cpt-insightspec-previews-nfr-offline-contract`

The OpenAPI document is emitted offline from the same route table the server mounts, and CI gates the committed document against drift.

### 6.2 NFR Exclusions

- No horizontal scaling requirements: preview provisioning is a low-rate operator action; a single replica suffices.
- No persistence or backup requirements: the cluster state is the data.

## 7. Public Library Interfaces

The service publishes an HTTP API only; its crate exposes no library surface to other components.

### 7.1 Public API Surface

#### Experiments REST API

The `v1` experiments endpoints (list, create, delete) described in the functional requirements, mounted by the API gateway under the service's public prefix. The committed OpenAPI document is the authoritative contract.

### 7.2 External Integration Contracts

#### Kubernetes API

Typed create/list/delete of `Deployment`, `Service`, and Gateway API `HTTPRoute` objects in the one configured namespace, using the in-cluster service account.

## 8. Use Cases

### 8.1 Publish a Branch Preview

- [ ] `p1` - **ID**: `cpt-insightspec-previews-usecase-publish`

An experiment author has a CI-built frontend image tag for their branch. They create an experiment with a name and that tag, receive the record with the experiment URL, wait for the status to become ready, and share the URL. When the review is done they delete the experiment -- or forget it, and the TTL sweep removes it.

## 9. Acceptance Criteria

- The three endpoints behave as specified against a cluster, and the created trio serves the frontend under `/exp/<name>` through the shared Gateway.
- The provisioned object shape matches the manual chart's render contract, pinned by unit tests.
- Invalid names, invalid tags, out-of-range TTLs, duplicate names, cap overruns, and unauthenticated calls are each refused with the documented canonical error.
- Expired experiments disappear without operator action.

## 10. Dependencies

- The shared Envoy Gateway and the prefix-serving frontend image contract (both already live).
- The gateway JWT verification pipeline shared by every insight backend service.
- Deploy wiring (namespace, RBAC, route) lands in the next phase; until then the service runs only where those are provided by hand.

## 11. Assumptions

- One preview host serves every experiment; only the path varies.
- Frontend branch images are published by CI under the product frontend repository with predictable tag shapes.
- The stand's authenticator gates whether an `/exp/` login return is honored; this service does not re-implement that gate.

## 12. Risks

- A service that mutates the cluster is an attractive escalation target; the mitigations are structural (tag-only input, fixed pod shape, one namespace, cap, TTL) plus namespace-scoped RBAC in the deploy phase.
- Leftover objects from a partially failed delete could block a later create of the same name; create refuses on conflict rather than adopting foreign objects.
