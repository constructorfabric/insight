---
status: draft
date: 2026-09-07
---

# Decomposition: Insight v3 Core

<!-- toc -->

- [1. Overview](#1-overview)
- [2. Entries](#2-entries)
  - [2.1 Data Ingestion - HIGH](#21-data-ingestion---high)
  - [2.2 Semantic Layer - HIGH](#22-semantic-layer---high)
  - [2.3 Widgets - HIGH](#23-widgets---high)
  - [2.4 Dashboards - HIGH](#24-dashboards---high)
  - [2.5 Alerts - MEDIUM](#25-alerts---medium)
  - [2.6 AI - HIGH](#26-ai---high)
  - [2.7 Data Access - HIGH](#27-data-access---high)
  - [2.8 Authoring over MCP - HIGH](#28-authoring-over-mcp---high)
- [3. Feature Dependencies](#3-feature-dependencies)

<!-- /toc -->

## 1. Overview

Eight features: data in, data out, metrics over it, widgets, dashboards, alerts, a chat over all of
it, and an agent authoring the same things over MCP.

The platform-usage requirements in [PRD §5.7](./PRD.md#57-platform-usage) are not decomposed yet.

## 2. Entries

**Overall implementation status:**

- [ ] `p1` - **ID**: `cpt-insightspec-v3-status-overall`

### 2.1 Data Ingestion - HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-v3-feature-data-ingestion`

- **Purpose**: Get data in.

- **Depends On**: None

- **Requirements Covered**:

  - [x] `p1` - `cpt-insightspec-v3-fr-accept-data`
  - [x] `p1` - `cpt-insightspec-v3-fr-write-connectors`
  - [ ] `p2` - `cpt-insightspec-v3-fr-first-class-connectors`

- **Data**:

  - `cpt-insightspec-v3-dbtable-raw-data`

### 2.2 Semantic Layer - HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-v3-feature-semantic-layer`

- **Purpose**: Create metrics based on ingested data, and edit, find and delete them.

- **Depends On**: 2.1

- **Requirements Covered**:

  - [x] `p1` - `cpt-insightspec-v3-fr-create-metrics`
  - [ ] `p2` - `cpt-insightspec-v3-fr-metrics-for-stand`

### 2.3 Widgets - HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-v3-feature-widgets`

- **Purpose**: Create widgets in realtime, per user, and edit, find and delete them.

- **Depends On**: 2.2

- **Requirements Covered**:

  - [x] `p1` - `cpt-insightspec-v3-fr-create-widgets`

- **Out of scope**:
  - Sharing widgets, which comes later

### 2.4 Dashboards - HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-v3-feature-dashboards`

- **Purpose**: Create dashboards in realtime, per user, and edit, find and delete them.

- **Depends On**: 2.3

- **Requirements Covered**:

  - [x] `p1` - `cpt-insightspec-v3-fr-create-dashboards`

- **Out of scope**:
  - Sharing dashboards, which comes later

### 2.5 Alerts - MEDIUM

- [ ] `p2` - **ID**: `cpt-insightspec-v3-feature-alerts`

- **Purpose**: Create alerts in realtime, delivered to Zulip.

- **Depends On**: 2.2

- **Requirements Covered**:

  - [ ] `p2` - `cpt-insightspec-v3-fr-create-alerts`

- **Out of scope**:
  - Destinations other than Zulip

### 2.6 AI - HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-v3-feature-ai`

- **Purpose**: A chat over the data, reading context a team adds for itself.

- **Depends On**: 2.2, 2.3, 2.4, 2.5

- **Requirements Covered**:

  - [x] `p1` - `cpt-insightspec-v3-fr-ai-answer`
  - [x] `p1` - `cpt-insightspec-v3-fr-ai-create`
  - [ ] `p2` - `cpt-insightspec-v3-fr-ai-context`

### 2.7 Data Access - HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-v3-feature-data-access`

- **Purpose**: Get data out, to whoever their role says may read it.

- **Depends On**: 2.1, 2.2

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-v3-fr-read-data`
  - [ ] `p2` - `cpt-insightspec-v3-fr-download-report`

### 2.8 Authoring over MCP - HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-v3-feature-mcp-authoring`

- **Purpose**: Let an agent author what a person authors.

- **Depends On**: 2.2, 2.3, 2.4, 2.7

- **Requirements Covered**: TBD — the PRD describes the chat, not an agent-facing surface.

## 3. Feature Dependencies

```text
2.1 Data Ingestion
 |
 +-- 2.7 Data Access
 +-- 2.2 Semantic Layer
      |
      +-- 2.5 Alerts
      +-- 2.3 Widgets --- 2.4 Dashboards
                           |
                           +-- 2.6 AI (over 2.2-2.5)
                           +-- 2.8 Authoring over MCP (over 2.2-2.4, 2.7)
```
