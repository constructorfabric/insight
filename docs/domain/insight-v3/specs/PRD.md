---
status: draft
date: 2026-09-07
---

> [!NOTE]
> Keep this PRD very brief — short sections, no filler.

# PRD -- Insight v3 Core Service

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
  - [5.1 Data Ingestion](#51-data-ingestion)
  - [5.2 Identity Resolution](#52-identity-resolution)
  - [5.3 Access Control](#53-access-control)
  - [5.4 Metrics, Widgets and Dashboards](#54-metrics-widgets-and-dashboards)
  - [5.5 AI](#55-ai)
  - [5.6 Ingestion Status](#56-ingestion-status)
  - [5.7 Platform Usage](#57-platform-usage)
  - [5.8 Alerts](#58-alerts)
  - [5.9 Query Optimization](#59-query-optimization)
  - [5.10 Data Access](#510-data-access)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 Quality Vector Analysis](#61-quality-vector-analysis)
  - [6.2 NFR Inclusions](#62-nfr-inclusions)
  - [6.3 NFR Exclusions](#63-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

### 1.2 Background / Problem Statement

### 1.3 Goals (Business Outcomes)

- Easy to add new data, or write a connector to get new data.
- Easy to create metrics.
- Easy to create new widgets.
- Easy to create new dashboards.
- Easy to create new alerts.
- Ability to use your own AI (LLM, Claude) via CLI/MCP.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Quality vector | One of the five axes Constructor Fabric measures quality on: Efficiency, Reliability, Performance, Security, Versatility, in that priority order |
| North-star metric | The single primary indicator for a quality vector |
| Reference organization | The synthetic sizing baseline used when measuring against a threshold |
| Dataset | A named, described set of ingested records at one grain; the only source a custom metric reads |
| Field role | What a dataset field is for: a dimension (group and filter by it), a measurable (aggregate it) or a time field (window by it) |
| Row identity | The dataset fields that make two records the same record |

## 2. Actors

### 2.1 Human Actors

#### Dashboard Author

**ID**: `cpt-insightspec-v3-actor-dashboard-author`

**Role**: Creates metrics, widgets and dashboards.
**Needs**: TBD

#### Instance Administrator

**ID**: `cpt-insightspec-v3-actor-administrator`

**Role**: Creates and removes datasets; the only role the custom surfaces are open to.
**Needs**: To see what data exists on the stand, describe it once, and remove what should not be there.

### 2.2 System Actors

#### External Connector

**ID**: `cpt-insightspec-v3-actor-external-connector`

**Role**: Sends data in via the API.

#### Internal Connector

**ID**: `cpt-insightspec-v3-actor-internal-connector`

**Role**: An Insight connector, sending the data it syncs over the same contract.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

TBD

## 4. Scope

### 4.1 In Scope

TBD

### 4.2 Out of Scope

TBD

## 5. Functional Requirements

### 5.1 Data Ingestion

#### Accept Data Over the API

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-accept-data`

The system **MUST** accept data of any shape sent to it over the API.

**Actors**: `cpt-insightspec-v3-actor-external-connector`, `cpt-insightspec-v3-actor-internal-connector`

#### Write Connectors in Realtime

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-write-connectors`

The system **MUST** let a connector be written and start sending data without a release.

**Actors**: `cpt-insightspec-v3-actor-external-connector`, `cpt-insightspec-v3-actor-internal-connector`

#### First-Class Connectors

- [ ] `p2` - **ID**: `cpt-insightspec-v3-fr-first-class-connectors`

The system **MUST** support first-class connectors, such as a GitHub mirror.

**Actors**: `cpt-insightspec-v3-actor-external-connector`, `cpt-insightspec-v3-actor-internal-connector`

#### Create a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-create-dataset`

The system **MUST** let an administrator create a dataset, in the portal and over the API, declaring its name, title, description, fields with their payload key, type and role, its default time field and its row identity.

**Actors**: `cpt-insightspec-v3-actor-administrator`

#### Remove a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-remove-dataset`

The system **MUST** let an administrator remove a dataset together with its records, in the portal and over the API, and **MUST** refuse while a metric reads it, naming the metrics.

**Actors**: `cpt-insightspec-v3-actor-administrator`

#### View a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-view-dataset`

The system **MUST** show a dataset's declaration, a preview of its latest records and the metrics that read it in the portal.

**Actors**: `cpt-insightspec-v3-actor-administrator`, `cpt-insightspec-v3-actor-dashboard-author`

#### Ingest Into a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-ingest-into-dataset`

The system **MUST** accept a record only into a dataset that already exists, store it whole whatever its shape, and refuse a record addressed to a dataset that does not exist without creating anything.

**Actors**: `cpt-insightspec-v3-actor-external-connector`, `cpt-insightspec-v3-actor-internal-connector`

### 5.2 Identity Resolution

TBD

### 5.3 Access Control

TBD

### 5.4 Metrics, Widgets and Dashboards

#### Create Metrics in Realtime

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-create-metrics`

The system **MUST** let a metric be created from ingested data in realtime.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Create Metrics for a Stand

- [ ] `p2` - **ID**: `cpt-insightspec-v3-fr-metrics-for-stand`

The system **MUST** let metrics be created for a stand.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Create Widgets in Realtime

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-create-widgets`

The system **MUST** let a widget be created in realtime.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Create Dashboards in Realtime

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-create-dashboards`

The system **MUST** let a dashboard be created in realtime.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Metrics Read Datasets

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-metrics-over-datasets`

A metric **MUST** name a dataset and refer to its declared fields by name, never a table; the system **MUST** refuse a metric over an unknown dataset or an undeclared field.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

### 5.5 AI

#### Answer a Question

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-ai-answer`

The chat **MUST** answer a specific question about the data.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Create Through the Chat

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-ai-create`

The chat **MUST** be able to create a metric, a widget, a dashboard or an alert.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Add Your Own Context

- [ ] `p2` - **ID**: `cpt-insightspec-v3-fr-ai-context`

The chat **MUST** read context a team writes for itself — what its metrics mean, which tables to
prefer, the words it uses — and answer in those terms.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Read Datasets in the Chat

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-assistant-reads-datasets`

The chat and the MCP tools **MUST** be told the datasets — their descriptions and declared fields — and **MUST** answer and build from datasets only.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

### 5.6 Ingestion Status

TBD

### 5.7 Platform Usage

#### Popular Dashboards and Widgets

- [ ] `p2` - **ID**: `cpt-insightspec-v3-fr-usage-popular`

The system **MUST** report which dashboards and widgets are opened, how often, and by how many people.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Popular Questions

- [ ] `p2` - **ID**: `cpt-insightspec-v3-fr-usage-questions`

The system **MUST** report which questions are asked of the chat.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

### 5.8 Alerts

#### Create Alerts in Realtime

- [ ] `p2` - **ID**: `cpt-insightspec-v3-fr-create-alerts`

The system **MUST** let an alert be created in realtime.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

### 5.9 Query Optimization

TBD

### 5.10 Data Access

#### Read Data Over the API

- [ ] `p1` - **ID**: `cpt-insightspec-v3-fr-read-data`

The system **MUST** let stored data be read out over the API.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

#### Download Data as a Report

- [ ] `p2` - **ID**: `cpt-insightspec-v3-fr-download-report`

The system **MUST** let data be downloaded manually, in a report.

**Actors**: `cpt-insightspec-v3-actor-dashboard-author`

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Vector | What is at stake here | Where it is answered |
|---|---|---|
| Efficiency | A stand runs this beside everything else Insight deploys, so a service that stores definitions and reads records must not raise the footprint an installation already budgets for | `cpt-insightspec-v3-nfr-efficiency` |
| Reliability | Data arrives from senders that cannot retry forever, and a reader who cannot tell a wrong number from a right one stops trusting every number | `cpt-insightspec-v3-nfr-reliability` |
| Performance | A dashboard nobody waits for is a dashboard nobody opens | `cpt-insightspec-v3-nfr-performance` |
| Security | The data describes people, and the surfaces that read it are open to whoever the roles say | `cpt-insightspec-v3-nfr-security` |
| Versatility | The product's claim is that new data, a new metric and a new board arrive without a release | `cpt-insightspec-v3-nfr-versatility` |

### 6.2 NFR Inclusions

One requirement per quality vector, in the priority order the vectors carry.

#### Efficiency

- [ ] `p1` - **ID**: `cpt-insightspec-v3-nfr-efficiency`

The system **MUST** run within the deployment footprint already recommended for Insight, without raising it.

**Threshold**: CPU, RAM and disk per environment no higher than the previous release, measured against the reference organization.

#### Reliability

- [ ] `p1` - **ID**: `cpt-insightspec-v3-nfr-reliability`

The system **MUST** stay available for ingest and read.

**Threshold**: at least 99.9% uptime per service.

#### Performance

- [ ] `p1` - **ID**: `cpt-insightspec-v3-nfr-performance`

Dashboards **MUST** open quickly at reference-organization scale.

**Threshold**: dashboard open p95 under 2 s; first paint p95 (LCP) at most 2,500 ms.

#### Security

- [ ] `p1` - **ID**: `cpt-insightspec-v3-nfr-security`

Published artifacts **MUST** carry no critical findings.

**Threshold**: zero critical findings across dependency, code, secret and image scans; a critical finding blocks the merge.

#### Versatility

- [ ] `p1` - **ID**: `cpt-insightspec-v3-nfr-versatility`

Adding data, a widget type, a dashboard or an alert **MUST NOT** require changes to service or renderer code.

**Threshold**: a new data source lands through the ingest contract alone; a new widget type ships as a schema change alone; a new dashboard and a new alert are created through the product, with no code change.


### 6.3 NFR Exclusions

None. Every project-default NFR applies.


## 7. Public Library Interfaces

### 7.1 Public API Surface

TBD

### 7.2 External Integration Contracts

TBD

## 8. Use Cases

#### TBD

- [ ] `p2` - **ID**: `cpt-insightspec-v3-usecase-tbd`

**Actor**: `cpt-insightspec-v3-actor-dashboard-author`

**Preconditions**:
- TBD

**Main Flow**:
1. TBD

**Postconditions**:
- TBD

#### Describe a Dataset and Build a Board Over It

- [ ] `p1` - **ID**: `cpt-insightspec-v3-usecase-dataset-to-dashboard`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Preconditions**:
- An external system has records to send and knows the stand's instance token
- The administrator is signed in to the portal

**Main Flow**:
1. The administrator creates a dataset in the Custom zone: a title, a description, the fields with their roles, the time field, the row identity
2. The external system sends its records into the dataset
3. The administrator opens the dataset page and checks the preview of the latest records
4. The administrator asks the assistant for a dashboard over the dataset; the assistant builds the metric, the widgets and the dashboard from the declaration

**Postconditions**:
- The dashboard renders from the dataset's records, re-sent records counted once
- The dataset cannot be removed while the dashboard's metric reads it

## 9. Acceptance Criteria

The three Constructor Fabric quality gates, enforced in CI and blocking:

- [ ] Tests and metrics exist and pass for every one of the five quality vectors
- [ ] New-code line coverage above 80%, and no change lowers a component's coverage
- [ ] Metrics collected and compared release to release: a metric either improves or does not degrade, and any degradation carries a documented, owned waiver

## 10. Dependencies

TBD

## 11. Assumptions

TBD

## 12. Risks
