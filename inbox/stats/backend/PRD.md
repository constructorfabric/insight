# PRD — Git Stats Backend

## Table of Contents

1. [Overview](#1-overview)
   - [Purpose](#11-purpose)
   - [Background / Problem Statement](#12-background--problem-statement)
   - [Goals (Business Outcomes)](#13-goals-business-outcomes)
   - [Glossary](#14-glossary)
2. [Actors](#2-actors)
   - [Human Actors](#21-human-actors)
   - [System Actors](#22-system-actors)
3. [Operational Concept & Environment](#3-operational-concept--environment)
   - [Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
4. [Scope](#4-scope)
   - [In Scope](#41-in-scope)
   - [Out of Scope](#42-out-of-scope)
5. [Functional Requirements](#5-functional-requirements)
   - [Authentication & Authorization](#51-authentication--authorization)
   - [User Management](#52-user-management)
   - [Permission Management](#53-permission-management)
   - [Data Proxy](#54-data-proxy)
   - [Performance Monitoring](#55-performance-monitoring)
   - [Configuration Management](#56-configuration-management)
6. [Non-Functional Requirements](#6-non-functional-requirements)
   - [NFR Inclusions](#61-nfr-inclusions)
   - [GDPR Compliance](#62-gdpr-compliance)
   - [Explicitly Not Applicable Requirements](#63-explicitly-not-applicable-requirements)
7. [Public Library Interfaces](#7-public-library-interfaces)
   - [Public API Surface](#71-public-api-surface)
   - [External Integration Contracts](#72-external-integration-contracts)
8. [Use Cases](#8-use-cases)
   - [Authentication & User Management](#81-authentication--user-management)
   - [Daily Analytics Workflows](#82-daily-analytics-workflows)
   - [Weekly Review Patterns](#83-weekly-review-patterns)
   - [Performance & Monitoring](#84-performance--monitoring)
9. [Acceptance Criteria](#9-acceptance-criteria)
10. [Dependencies](#10-dependencies)
11. [Assumptions](#11-assumptions)
12. [Risks](#12-risks)

## 1. Overview

### 1.1 Purpose

The Git Stats Backend is a REST API server that provides authentication, authorization, user management, and data proxy services for the Git Stats Dashboard. It serves as the central authentication gateway and data access layer, enabling secure access to analytics data while managing user permissions and integrating with enterprise SSO systems.

### 1.2 Background / Problem Statement

Git statistics and analytics systems require secure access control, especially in enterprise environments where data contains sensitive information about developer activity, code contributions, and organizational metrics. Without a proper authentication and authorization layer, analytics dashboards would expose sensitive data to unauthorized users or require each client to implement authentication independently.

The Git Stats Dashboard needs a backend service that can:
- Authenticate users via enterprise SSO (ZTA Passport/OIDC) and local credentials
- Manage user roles and granular permissions for pages and charts
- Proxy and secure access to analytics database
- Validate users against enterprise directory (Panopticum)
- Provide performance monitoring for database queries
- Support both production SSO and development local authentication

### 1.3 Goals (Business Outcomes)

- Enable secure enterprise SSO authentication with 99.9% uptime for authentication services
- Reduce unauthorized data access incidents to zero through role-based access control
- Provide sub-second API response times (p95 < 500ms) for user management operations
- Support 100+ concurrent users with horizontal scalability
- Enable granular permission management reducing admin overhead by 50%

### 1.4 Glossary

| Term | Definition |
|------|------------|
| ZTA Passport | Zero Trust Architecture Passport - enterprise OIDC authentication provider |
| OIDC | OpenID Connect - authentication protocol built on OAuth 2.0 |
| Panopticum | Enterprise user directory and organizational structure system |
| Analytics Database | Database system used for analytics data storage |
| RBAC | Role-Based Access Control - permission system based on user roles |
| SSO | Single Sign-On - centralized authentication mechanism |

## 2. Actors

### 2.1 Human Actors

#### Dashboard User

**ID**: `cpt-gitstats-actor-dashboard-user`

**Role**: End user accessing the Git Stats Dashboard to view analytics and metrics. May have restricted access based on assigned permissions.

**Needs**: 
- Authenticate via SSO or local credentials
- View permitted pages and charts
- Access analytics data within authorization scope
- Manage personal settings and preferences

#### System Administrator

**ID**: `cpt-gitstats-actor-admin`

**Role**: Administrator responsible for managing users, roles, and permissions within the Git Stats system.

**Needs**:
- Create and manage user accounts
- Assign and modify user roles
- Configure page and chart permissions
- Monitor system health and query performance
- Search and validate users in Panopticum
- Manage author aliases for data normalization

### 2.2 System Actors

#### Frontend Application

**ID**: `cpt-gitstats-actor-frontend`

**Role**: React-based dashboard application that consumes backend APIs to display analytics and manage user sessions.

**Needs**:
- Authenticate users and maintain sessions
- Retrieve current user permissions
- Fetch analytics data via proxy endpoints
- Manage user settings

#### ZTA Passport (OIDC Provider)

**ID**: `cpt-gitstats-actor-zta-passport`

**Role**: Enterprise SSO provider handling OIDC authentication flows and token validation.

**Needs**:
- Receive authentication requests
- Validate user credentials
- Issue authentication tokens
- Handle callback redirects

#### Panopticum Service

**ID**: `cpt-gitstats-actor-panopticum`

**Role**: External enterprise directory service providing user validation and organizational metadata.

**Needs**:
- Respond to user search queries
- Provide user organizational information
- Validate user existence

#### Analytics Database

**ID**: `cpt-gitstats-actor-analytics-database`

**Role**: Analytics database storing git statistics, commits, pull requests, and metrics data.

**Needs**:
- Execute parameterized queries
- Return analytics results
- Respond to health checks

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires server-side runtime environment
- Requires network connectivity to analytics database
- Requires network connectivity to ZTA Passport OIDC endpoints
- Requires network connectivity to Panopticum API
- Requires persistent storage for user data and permissions
- Supports containerization for deployment
- CORS configuration required for frontend integration

## 4. Scope

### 4.1 In Scope

- SSO authentication via OIDC (ZTA Passport)
- Local username/password authentication for development
- User profile management with Panopticum integration
- Role-based access control (admin/user roles)
- Granular page and chart permissions
- Analytics data proxy with structured endpoints
- Query performance monitoring and logging
- Author alias management for data normalization
- User settings and preferences
- API token authentication for programmatic access
- Health check and status endpoints
- Admin interface for system management
- Automated testing with 90%+ code coverage

### 4.2 Out of Scope

- Direct analytics database schema management (handled by separate ETL processes)
- Git repository data collection and ingestion
- Frontend UI components and visualization
- Email notification system
- Server-side rendering (SSR) for frontend
- Real-time WebSocket connections
- GraphQL API (REST only)
- Multi-tenancy support (single organization deployment)
- Advanced analytics computation (delegated to analytics database)
- Multi-language API responses (English only)
- Localized error messages
- Internationalization support (i18n/l10n)

## 5. Functional Requirements

### 5.1 Authentication & Authorization

#### SSO Authentication

- [ ] `p1` - **ID**: `cpt-gitstats-fr-sso-auth`

The system **MUST** support OIDC authentication flow with ZTA Passport, including authorization code exchange, token validation, and session establishment.

**Rationale**: Enterprise security policy requires centralized SSO authentication for all internal applications.

**Actors**: `cpt-gitstats-actor-dashboard-user`, `cpt-gitstats-actor-zta-passport`

#### Local Authentication

- [ ] `p2` - **ID**: `cpt-gitstats-fr-local-auth`

The system **MUST** support local username/password authentication as a fallback mechanism for development and emergency access.

**Rationale**: Development environments may not have access to production SSO, and emergency access is needed if SSO is unavailable.

**Actors**: `cpt-gitstats-actor-dashboard-user`

#### Session Management

- [ ] `p1` - **ID**: `cpt-gitstats-fr-session-mgmt`

The system **MUST** maintain secure user sessions with configurable timeout and support logout functionality.

**Rationale**: Secure session management prevents unauthorized access and ensures proper cleanup of authentication state.

**Actors**: `cpt-gitstats-actor-dashboard-user`

#### Authentication Status Check

- [ ] `p1` - **ID**: `cpt-gitstats-fr-auth-status`

The system **MUST** provide an endpoint to check current authentication status and retrieve user information.

**Rationale**: Frontend needs to determine authentication state on page load and refresh.

**Actors**: `cpt-gitstats-actor-frontend`

#### API Token Authentication

- [ ] `p2` - **ID**: `cpt-gitstats-fr-api-token`

The system **MUST** support API token-based authentication for programmatic access with token creation, revocation, and expiration.

**Rationale**: Automated tools and scripts need non-interactive authentication mechanism.

**Actors**: `cpt-gitstats-actor-dashboard-user`

### 5.2 User Management

#### User Profile Creation

- [ ] `p1` - **ID**: `cpt-gitstats-fr-user-create`

The system **MUST** allow administrators to create user profiles by searching and selecting users from Panopticum, automatically importing organizational metadata.

**Rationale**: User provisioning must integrate with enterprise directory to ensure data consistency and validation.

**Actors**: `cpt-gitstats-actor-admin`, `cpt-gitstats-actor-panopticum`

#### User Profile Retrieval

- [ ] `p1` - **ID**: `cpt-gitstats-fr-user-retrieve`

The system **MUST** provide endpoints to retrieve individual user details and list all users with pagination support.

**Rationale**: Administrators need to view and manage user accounts efficiently.

**Actors**: `cpt-gitstats-actor-admin`

#### Role Management

- [ ] `p1` - **ID**: `cpt-gitstats-fr-role-mgmt`

The system **MUST** support assigning and updating user roles (admin/user) with immediate effect on permissions.

**Rationale**: Role changes must take effect immediately to ensure proper access control.

**Actors**: `cpt-gitstats-actor-admin`

#### User Activation Control

- [ ] `p1` - **ID**: `cpt-gitstats-fr-user-activation`

The system **MUST** allow administrators to enable or disable user accounts without deletion.

**Rationale**: Temporary access suspension is needed for offboarding or security incidents without losing user data.

**Actors**: `cpt-gitstats-actor-admin`

#### User Deletion

- [ ] `p2` - **ID**: `cpt-gitstats-fr-user-delete`

The system **MUST** allow administrators to permanently delete user accounts and associated data.

**Rationale**: GDPR compliance and data cleanup require permanent user removal capability.

**Actors**: `cpt-gitstats-actor-admin`

#### Current User Information

- [ ] `p1` - **ID**: `cpt-gitstats-fr-current-user`

The system **MUST** provide an endpoint returning current authenticated user's profile, role, and permissions.

**Rationale**: Frontend needs complete user context to render appropriate UI and enforce client-side permission checks.

**Actors**: `cpt-gitstats-actor-frontend`, `cpt-gitstats-actor-dashboard-user`

### 5.3 Permission Management

#### Page Permission Assignment

- [ ] `p1` - **ID**: `cpt-gitstats-fr-page-perms`

The system **MUST** allow administrators to configure which pages are visible to each user, with default all-visible behavior.

**Rationale**: Different users need access to different analytics views based on their role and responsibilities.

**Actors**: `cpt-gitstats-actor-admin`

#### Chart Permission Assignment

- [ ] `p1` - **ID**: `cpt-gitstats-fr-chart-perms`

The system **MUST** allow administrators to configure which charts within pages are visible to each user.

**Rationale**: Granular control enables hiding sensitive metrics while allowing access to general analytics.

**Actors**: `cpt-gitstats-actor-admin`

#### Permission Retrieval

- [ ] `p1` - **ID**: `cpt-gitstats-fr-perm-retrieve`

The system **MUST** provide endpoints to retrieve user permissions and available pages/charts.

**Rationale**: Frontend needs permission data to render appropriate UI elements and enforce access control.

**Actors**: `cpt-gitstats-actor-frontend`, `cpt-gitstats-actor-admin`

#### Permission Enforcement

- [ ] `p1` - **ID**: `cpt-gitstats-fr-perm-enforce`

The system **MUST** enforce permission checks on all data proxy endpoints, returning 403 Forbidden for unauthorized access.

**Rationale**: Backend must validate permissions to prevent unauthorized data access regardless of frontend controls.

**Actors**: `cpt-gitstats-actor-dashboard-user`

### 5.4 Data Proxy

#### Analytics Database Health Check

- [ ] `p1` - **ID**: `cpt-gitstats-fr-health-check`

The system **MUST** provide a health check endpoint verifying analytics database connectivity and returning appropriate status codes.

**Rationale**: Monitoring systems need to detect database connectivity issues for alerting and failover.

**Actors**: `cpt-gitstats-actor-frontend`

#### Structured Query Endpoints

- [ ] `p1` - **ID**: `cpt-gitstats-fr-structured-queries`

The system **MUST** provide structured REST endpoints for analytics queries (commits, pull requests, features, reports, etc.) with parameterized inputs and validated outputs.

**Rationale**: Structured endpoints prevent SQL injection and provide type-safe data access.

**Actors**: `cpt-gitstats-actor-frontend`, `cpt-gitstats-actor-dashboard-user`

#### Query Parameter Validation

- [ ] `p1` - **ID**: `cpt-gitstats-fr-param-validation`

The system **MUST** validate all query parameters (date ranges, emails, filters) before executing analytics database queries.

**Rationale**: Input validation prevents injection attacks and ensures data integrity.

**Actors**: `cpt-gitstats-actor-frontend`

#### Pagination Support

- [ ] `p2` - **ID**: `cpt-gitstats-fr-pagination`

The system **MUST** support pagination for large result sets with configurable page size and offset parameters.

**Rationale**: Large datasets must be paginated to prevent memory exhaustion and improve response times.

**Actors**: `cpt-gitstats-actor-frontend`

#### Error Handling

- [ ] `p1` - **ID**: `cpt-gitstats-fr-error-handling`

The system **MUST** return appropriate HTTP status codes and error messages for database errors, validation failures, and permission denials.

**Rationale**: Proper error handling enables frontend to provide meaningful feedback and aids debugging.

**Actors**: `cpt-gitstats-actor-frontend`

### 5.5 Performance Monitoring

#### Slow Query Logging

- [ ] `p2` - **ID**: `cpt-gitstats-fr-slow-query-log`

The system **MUST** log queries exceeding configurable threshold (default 3 seconds) with sanitized parameters and execution time.

**Rationale**: Performance monitoring enables identification and optimization of slow queries.

**Actors**: `cpt-gitstats-actor-admin`

#### Query Statistics

- [ ] `p2` - **ID**: `cpt-gitstats-fr-query-stats`

The system **MUST** provide admin endpoints to retrieve aggregated query statistics (count, average duration, slow queries) and reset statistics.

**Rationale**: Administrators need visibility into query performance patterns for optimization.

**Actors**: `cpt-gitstats-actor-admin`

#### PII Sanitization

- [ ] `p1` - **ID**: `cpt-gitstats-fr-pii-sanitization`

The system **MUST** sanitize email addresses and other PII in logs by masking characters (e.g., u***@domain.com).

**Rationale**: Log files must not expose sensitive user information for security and compliance.

**Actors**: `cpt-gitstats-actor-admin`

### 5.6 Panopticum Integration

#### User Search

- [ ] `p1` - **ID**: `cpt-gitstats-fr-panopticum-search`

The system **MUST** provide an endpoint to search Panopticum users by name or email with result caching.

**Rationale**: User provisioning requires searching enterprise directory to find and validate users.

**Actors**: `cpt-gitstats-actor-admin`, `cpt-gitstats-actor-panopticum`

#### Metadata Import

- [ ] `p1` - **ID**: `cpt-gitstats-fr-panopticum-import`

The system **MUST** import and store Panopticum user metadata (user_id, username, team, unit, organization) during user creation.

**Rationale**: Organizational context is needed for analytics filtering and user identification.

**Actors**: `cpt-gitstats-actor-admin`, `cpt-gitstats-actor-panopticum`

### 5.7 Author Alias Management

#### Alias Creation

- [ ] `p2` - **ID**: `cpt-gitstats-fr-alias-create`

The system **MUST** allow administrators to create author aliases mapping git commit authors to canonical user identities.

**Rationale**: Git commits may use different email addresses or names that need normalization for accurate analytics.

**Actors**: `cpt-gitstats-actor-admin`

#### Alias Retrieval

- [ ] `p2` - **ID**: `cpt-gitstats-fr-alias-retrieve`

The system **MUST** provide endpoints to list and retrieve author aliases with filtering and pagination.

**Rationale**: Administrators need to view and manage existing aliases for data quality.

**Actors**: `cpt-gitstats-actor-admin`

#### Bulk Alias Operations

- [ ] `p2` - **ID**: `cpt-gitstats-fr-alias-bulk`

The system **MUST** support bulk creation and deletion of author aliases for efficient management.

**Rationale**: Large-scale alias management requires batch operations to reduce administrative overhead.

**Actors**: `cpt-gitstats-actor-admin`

### 5.8 User Settings

#### Settings Storage

- [ ] `p2` - **ID**: `cpt-gitstats-fr-settings-storage`

The system **MUST** allow users to store and retrieve personal settings and preferences as JSON data.

**Rationale**: User experience requires persisting UI preferences, filters, and customizations.

**Actors**: `cpt-gitstats-actor-dashboard-user`

#### Settings Update

- [ ] `p2` - **ID**: `cpt-gitstats-fr-settings-update`

The system **MUST** provide an endpoint to update user settings with validation and merge support.

**Rationale**: Settings must be updatable without overwriting unrelated preferences.

**Actors**: `cpt-gitstats-actor-dashboard-user`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### API Response Time

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-response-time`

The system **MUST** respond to user management API requests within 500ms at p95 under normal load (100 concurrent users).

**Threshold**: p95 < 500ms for user management endpoints, p95 < 3s for complex analytics queries

**Rationale**: Responsive UI requires fast API responses for user interactions.

#### Authentication Availability

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-auth-availability`

The system **MUST** maintain 99.9% uptime for authentication endpoints (excluding planned maintenance).

**Threshold**: 99.9% uptime measured monthly

**Rationale**: Authentication is critical path for all user access; downtime blocks all dashboard usage.

#### Concurrent Users

- [ ] `p2` - **ID**: `cpt-gitstats-nfr-concurrent-users`

The system **MUST** support at least 100 concurrent authenticated users without degradation.

**Threshold**: 100 concurrent users with p95 response time < 500ms

**Rationale**: Enterprise deployment requires supporting multiple teams accessing analytics simultaneously.

#### Data Security

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-data-security`

The system **MUST** use HTTPS for all API communications and secure session cookies with HttpOnly and SameSite flags.

**Threshold**: 100% of API traffic over HTTPS, all cookies properly secured

**Rationale**: Security policy requires encrypted communications and secure cookie handling.

#### Code Coverage

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-test-coverage`

The system **MUST** maintain automated test coverage of at least 90% for all functional requirements.

**Threshold**: 90% code coverage measured by pytest-cov

**Rationale**: High test coverage ensures reliability and enables safe refactoring.

### 6.2 GDPR Compliance

The following GDPR compliance measures are implemented to ensure data protection and user privacy:

**Data Subject Rights**:
- **Right to erasure**: Implemented via user deletion endpoint (`cpt-gitstats-fr-user-delete`)
- **Right to access**: User can retrieve own profile via `/api/me/` endpoint
- **Right to rectification**: Admin can update user profiles via user management endpoints
- **Right to data portability**: Not applicable (minimal user data stored; no complex data structures)

**Data Protection Principles**:
- **Data minimization**: Only essential user data stored (username, email, role, permissions, settings)
- **Purpose limitation**: User data used exclusively for authentication, authorization, and system access control
- **Storage limitation**: No automatic retention policy; manual cleanup via deletion endpoint
- **Integrity and confidentiality**: HTTPS encryption for all communications, secure session management, PII sanitization in logs

**Privacy by Design**:
- PII sanitization in query logs (`cpt-gitstats-fr-pii-sanitization`)
- Secure cookie configuration with HttpOnly and Secure flags
- Session-based authentication with configurable timeout
- No unnecessary data collection or processing

**Not Applicable**:
- **Consent management**: Not required (legitimate interest basis for employee productivity monitoring within enterprise)
- **Data breach notification**: Delegated to infrastructure/security team per enterprise policy
- **Data processing agreements**: Panopticum and analytics database covered by existing enterprise data processing agreements
- **Cross-border data transfers**: Single-region deployment assumed; if multi-region needed, standard contractual clauses apply

### 6.3 Explicitly Not Applicable Requirements

The following quality characteristics from ISO/IEC 25010:2023 and Constructor Studio PRD checklist are **intentionally not applicable** to this backend PRD:

#### Safety (SAFE)

**Not applicable**: Backend REST API service with no physical interaction, medical devices, vehicles, or industrial control. No operations that could cause harm to people, property, or environment.

#### Accessibility (UX-PRD-002)

**Not applicable**: Backend REST API has no user interface. Accessibility requirements apply to frontend application consuming the API.

#### Inclusivity (UX-PRD-005)

**Not applicable**: Backend API service with no direct user interaction or user interface. Inclusivity considerations apply to frontend dashboard.

#### Detailed Infrastructure Specifications (OPS)

**Not applicable**: Infrastructure deployment (Kubernetes orchestration, load balancers, reverse proxies, CDN) delegated to infrastructure/DevOps team. PRD covers Docker containerization and environment configuration requirements only.

#### Internationalization (UX-PRD-003)

**Not applicable**: API responses and error messages in English only. No localization or multi-language support required for backend API.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### REST API v1

- [ ] `p1` - **ID**: `cpt-gitstats-interface-rest-api`

**Type**: REST API

**Stability**: stable

**Description**: HTTP REST API providing authentication, user management, permissions, and data proxy endpoints. All endpoints under `/api/` prefix with versioning (`/api/data/v1/`).

**Breaking Change Policy**: Major version bump required for breaking changes. Deprecation notices provided 90 days before removal.

#### Admin Interface

- [ ] `p2` - **ID**: `cpt-gitstats-interface-admin`

**Type**: Web interface

**Stability**: stable

**Description**: Admin interface at `/admin/` for system administration and data management.

**Breaking Change Policy**: No compatibility guarantees; admin interface may change between minor versions.

### 7.2 External Integration Contracts

#### OIDC Provider Contract

- [ ] `p1` - **ID**: `cpt-gitstats-contract-oidc`

**Direction**: required from client

**Protocol/Format**: OIDC/OAuth 2.0 (Authorization Code Flow)

**Compatibility**: Compatible with any OIDC 1.0 compliant provider

#### Panopticum API Contract

- [ ] `p1` - **ID**: `cpt-gitstats-contract-panopticum`

**Direction**: required from client

**Protocol/Format**: HTTP/REST JSON API

**Compatibility**: Requires Panopticum API v1 endpoints for user search

#### Analytics Database Contract

- [ ] `p1` - **ID**: `cpt-gitstats-contract-analytics-db`

**Direction**: required from client

**Protocol/Format**: HTTP interface with parameterized queries

**Compatibility**: Compatible with analytics database system

## 8. Use Cases

### 8.1 Authentication & User Management

#### SSO Login Flow

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-sso-login`

**Actor**: `cpt-gitstats-actor-dashboard-user`

**Preconditions**:
- User has valid enterprise SSO credentials
- ZTA Passport is operational
- User exists in Panopticum

**Main Flow**:
1. User clicks "Login with SSO" in frontend
2. Frontend redirects to `/oidc/authenticate/`
3. Backend redirects to ZTA Passport authorization endpoint
4. User authenticates with enterprise credentials
5. ZTA Passport redirects back with authorization code
6. Backend exchanges code for access token
7. Backend validates token and retrieves user info
8. Backend checks user exists in Panopticum
9. Backend creates/updates user profile
10. Backend establishes session
11. Backend redirects to frontend dashboard

**Postconditions**:
- User is authenticated with active session
- User profile is created/updated
- Frontend receives user permissions

**Alternative Flows**:
- **Panopticum validation fails**: User creation fails with error message
- **Token validation fails**: Redirect to error page with retry option

#### Admin Creates User

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-admin-create-user`

**Actor**: `cpt-gitstats-actor-admin`

**Preconditions**:
- Admin is authenticated with admin role
- Target user exists in Panopticum

**Main Flow**:
1. Admin searches Panopticum via `/api/panopticum/search/`
2. Admin selects user from search results
3. Admin submits user creation request to `/api/users/create/`
4. Backend validates user doesn't already exist
5. Backend creates user profile with Panopticum metadata
6. Backend assigns default role and permissions
7. Backend returns created user profile

**Postconditions**:
- New user profile exists in database
- User can authenticate and access dashboard

**Alternative Flows**:
- **User already exists**: Return error with existing user details
- **Panopticum user not found**: Return validation error

### 8.2 Data Access & Analytics

#### User Views Analytics Data

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-view-analytics`

**Actor**: `cpt-gitstats-actor-dashboard-user`

**Preconditions**:
- User is authenticated
- User has permission to view requested page/chart
- Analytics database is operational

**Main Flow**:
1. Frontend requests analytics data from structured endpoint
2. Backend validates user authentication
3. Backend checks user permissions for requested data
4. Backend validates query parameters
5. Backend executes parameterized analytics database query
6. Backend formats and returns results
7. Frontend renders visualization

**Postconditions**:
- User sees requested analytics data
- Query is logged for performance monitoring

**Alternative Flows**:
- **Permission denied**: Return 403 Forbidden
- **Analytics database error**: Return 503 Service Unavailable with error message
- **Invalid parameters**: Return 400 Bad Request with validation errors

### 8.3 Common User Journeys (Based on Real Usage Data)

#### Daily Dashboard Check-in

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-daily-checkin`

**Actor**: `cpt-gitstats-actor-dashboard-user`

**Frequency**: Daily (60% of sessions, 2-5 minutes)

**Preconditions**:
- User is authenticated
- User has Dashboard access permission

**Main Flow**:
1. Frontend requests Dashboard page data via `/api/data/v1/dashboard/`
2. Backend validates authentication and permissions
3. Backend retrieves Contributors Breakdown data
4. Backend retrieves Repository Metrics
5. Backend returns aggregated dashboard data
6. Frontend renders Dashboard with key metrics
7. User reviews Contributors Breakdown table
8. User identifies high/low performers

**Postconditions**:
- User has quick team pulse overview
- Session logged for analytics


#### AI Adoption Systematic Review

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-ai-adoption-review`

**Actor**: `cpt-gitstats-actor-dashboard-user`

**Frequency**: 2-5x per week (5-10 minutes, 53% of users)

**Preconditions**:
- User is authenticated
- User has AI Adoption page permission

**Main Flow**:
1. Frontend requests AI Adoption metrics via `/api/data/v1/ai-adoption/`
2. Backend retrieves Copilot charts data
3. Backend retrieves AI users table data
4. Backend retrieves Windsurf credits data
5. Backend retrieves AI active users metrics
6. Backend identifies missing Panopticum users
7. Backend returns comprehensive AI metrics
8. Frontend renders all AI charts
9. User systematically reviews each metric

**Postconditions**:
- User has complete AI adoption status
- Non-adopters identified for coaching


#### Commits Table Investigation

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-commits-investigation`

**Actor**: `cpt-gitstats-actor-dashboard-user`

**Frequency**: Daily to weekly (10-30 minutes, 100% of users)

**Preconditions**:
- User is authenticated
- User identified outlier in Dashboard

**Main Flow**:
1. User identifies outlier in Contributors Breakdown
2. Frontend requests commits data via `/api/data/v1/commits/`
3. Backend validates permissions
4. Backend executes filtered analytics database query with user/date/repo filters
5. Backend returns paginated commits data
6. Frontend renders commits table
7. User filters/sorts by user, repository, or date
8. User investigates specific commits
9. User opens User Details modal for deeper analysis

**Postconditions**:
- User has detailed commit-level insights
- Investigation workflow completed


#### Weekly Deep-Dive Analysis (Thursday Pattern)

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-weekly-deepdive`

**Actor**: `cpt-gitstats-actor-admin`

**Frequency**: Weekly (30-60 minutes, 70% of managers)

**Preconditions**:
- User is authenticated with admin role
- Weekly team meeting preparation needed

**Main Flow**:
1. Frontend requests comprehensive Dashboard data
2. Backend retrieves all dashboard metrics
3. User performs deep commits-table investigation
4. Frontend requests individual contributor analysis via `/api/users/`
5. Backend returns user performance data
6. User cross-navigates to Bitbucket metrics via `/api/data/v1/bitbucket/`
7. User reviews AI Adoption metrics
8. User checks Panopticum organizational data
9. User accesses Reports page for trend analysis
10. User prepares weekly summary

**Postconditions**:
- Weekly team review completed
- Summary prepared for team meeting


#### Individual Contributor Performance Review

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-individual-review`

**Actor**: `cpt-gitstats-actor-admin`

**Frequency**: 2-5x per week (15-30 minutes, 40% of managers)

**Preconditions**:
- User is authenticated with admin role
- 1-on-1 meeting preparation needed

**Main Flow**:
1. Frontend requests Users page data via `/api/users/`
2. Backend returns user list with performance metrics
3. User clicks on specific user
4. Frontend requests user details via `/api/users/<id>/`
5. Backend retrieves user profile and Panopticum data
6. Frontend requests user's commits via `/api/data/v1/commits/?user=<id>`
7. Frontend requests user's pull requests via `/api/data/v1/pull-requests/?user=<id>`
8. Backend returns comprehensive user activity data
9. User reviews commit history, language breakdown, repository contributions
10. User assesses AI tool usage patterns

**Postconditions**:
- Individual performance assessment completed
- 1-on-1 meeting preparation ready


#### Compliance Auditing Workflow

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-compliance-audit`

**Actor**: `cpt-gitstats-actor-admin`

**Frequency**: Regular (15-30 minutes)

**Preconditions**:
- User is authenticated
- User has Compliance page permission

**Main Flow**:
1. Frontend requests 4 Eyes Compliance data via `/api/data/v1/compliance/`
2. Backend retrieves compliance violations
3. Backend returns violations list
4. User reviews compliance violations
5. User clicks on violation to investigate
6. Frontend requests related commits via `/api/data/v1/commits/`
7. Backend filters commits related to violation
8. User investigates specific contributors
9. Frontend requests contributor details via `/api/users/<id>/`
10. User cross-references with Bitbucket PRs

**Postconditions**:
- Compliance violations reviewed
- Remediation actions identified

**Evidence**: 821 page views (4th most popular). High cross-navigation to commits and PRs suggests standard workflow.

#### Bitbucket PR Review Session

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-pr-review`

**Actor**: `cpt-gitstats-actor-dashboard-user`

**Frequency**: 1-3x per week (10-20 minutes, 70% of users)

**Preconditions**:
- User is authenticated
- User has Bitbucket page permission

**Main Flow**:
1. Frontend requests Bitbucket metrics via `/api/data/v1/bitbucket/`
2. Backend retrieves repository metrics table
3. Backend retrieves velocity trend data
4. Backend retrieves review time metrics
5. Backend retrieves reviewer performance data
6. Backend retrieves tech debt metrics
7. Backend returns comprehensive PR metrics
8. User systematically reviews all metrics
9. User identifies bottlenecks

**Postconditions**:
- PR velocity status assessed
- Bottlenecks identified for resolution

**Evidence**: Equal interaction counts (26, 26, 26, 26) indicate standardized PR review checklist. Weekly or bi-weekly pattern.

#### Weekend Planning/Executive Review

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-weekend-planning`

**Actor**: `cpt-gitstats-actor-admin`

**Frequency**: Weekends (15-30 minutes, 50% of managers)

**Preconditions**:
- User is authenticated
- High-level planning needed

**Main Flow**:
1. Frontend requests Dashboard overview
2. Backend returns high-level metrics only
3. User reviews Reports page via `/api/data/v1/reports/`
4. Backend returns trend data
5. User checks Scorecard via `/api/data/v1/scorecard/`
6. User reviews AI Adoption high-level metrics
7. User plans upcoming week priorities

**Postconditions**:
- Weekly planning completed
- Priorities identified for next week

**Evidence**: Weekend activity (10-34% of total) focused on high-level pages (Dashboard, Reports, Scorecard) rather than detailed commit analysis.

#### Late-Night Commit Investigation (Technical Lead Pattern)

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-latenight-investigation`

**Actor**: `cpt-gitstats-actor-admin`

**Frequency**: Almost nightly (15-30 minutes, 20% of users)

**Preconditions**:
- User is authenticated with technical lead role
- Code review responsibilities

**Main Flow**:
1. Frontend requests Commits Table directly via `/api/data/v1/commits/`
2. Backend executes detailed commit query
3. User investigates specific commits in detail
4. Frontend requests Dashboard Contributors data for cross-reference
5. User reviews Bitbucket repository metrics
6. User performs deep technical analysis

**Postconditions**:
- Code quality review completed
- Issues identified for follow-up

**Evidence**: Late-night pattern (00:00-01:00 UTC peak, 44% of activity) with commits-table focus. Suggests detailed code review work.

### 8.4 Performance & Monitoring

#### Slow Query Detection and Logging

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-slow-query-detection`

**Actor**: `cpt-gitstats-actor-admin`

**Preconditions**:
- Query execution exceeds threshold (default 3 seconds)
- Logging system is operational

**Main Flow**:
1. Backend receives analytics query request
2. Backend starts query timer
3. Backend executes ClickHouse query
4. Query execution exceeds threshold
5. Backend sanitizes PII in query parameters
6. Backend logs slow query with duration and sanitized params
7. Backend increments query statistics counters
8. Backend returns query results to frontend

**Postconditions**:
- Slow query logged with WARNING level
- Query statistics updated
- Admin can review via `/api/data/v1/admin/query-stats`

#### Admin Reviews Query Performance

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-admin-query-stats`

**Actor**: `cpt-gitstats-actor-admin`

**Preconditions**:
- User is authenticated with admin role
- Query statistics data exists

**Main Flow**:
1. Admin requests query statistics via `/api/data/v1/admin/query-stats`
2. Backend validates admin permissions
3. Backend aggregates query statistics (count, avg duration, slow queries)
4. Backend returns statistics data
5. Frontend renders performance dashboard
6. Admin identifies slow endpoints
7. Admin can reset statistics via `/api/data/v1/admin/query-stats/reset`

**Postconditions**:
- Query performance insights obtained
- Optimization targets identified

## 9. Acceptance Criteria

- [ ] SSO authentication successfully authenticates users via ZTA Passport with session establishment
- [ ] Local authentication provides fallback access for development environments
- [ ] Administrators can create, update, and manage user accounts with Panopticum integration
- [ ] Role-based access control enforces admin/user permissions on all endpoints
- [ ] Page and chart permissions correctly filter visible content for users
- [ ] All data proxy endpoints validate permissions and return 403 for unauthorized access
- [ ] ClickHouse queries use parameterized inputs preventing SQL injection
- [ ] Slow queries are logged with sanitized parameters when exceeding threshold
- [ ] API response times meet p95 < 500ms for user management operations
- [ ] Test coverage exceeds 90% for all functional requirements
- [ ] System handles 100 concurrent users without degradation
- [ ] All API communications use HTTPS with secure cookie configuration

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| ZTA Passport (OIDC) | Enterprise SSO authentication provider | p1 |
| Panopticum API | User directory and organizational metadata | p1 |
| ClickHouse Database | Analytics data storage and query engine | p1 |
| Django 5.2+ | Web framework and ORM | p1 |
| Django REST Framework | REST API framework | p1 |
| Authlib | OIDC/OAuth client library | p1 |
| clickhouse-connect | ClickHouse Python client | p1 |
| SQLite | User data persistence | p1 |
| Frontend Application | React dashboard consuming APIs | p1 |

## 11. Assumptions

- ZTA Passport OIDC endpoints are accessible from backend deployment environment
- Panopticum API is accessible and provides user search functionality
- ClickHouse database is pre-populated with analytics data by separate ETL processes
- Frontend handles CORS preflight requests appropriately
- Network connectivity is reliable between backend and external services
- SSL/TLS certificates are properly configured for HTTPS
- Django SECRET_KEY is securely managed in production environment
- Database backups are handled by infrastructure/operations team
- Single organization deployment (no multi-tenancy required)

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| ZTA Passport downtime blocks all SSO logins | HIGH - Users cannot access dashboard | Implement local authentication fallback; monitor SSO health; establish SLA with SSO team |
| Panopticum API unavailable during user creation | MEDIUM - Cannot create new users | Cache Panopticum data; allow manual user creation with admin approval |
| ClickHouse performance degradation | HIGH - Slow dashboard experience | Implement query timeout; add query performance monitoring; optimize slow queries |
| Session hijacking via XSS/CSRF | HIGH - Unauthorized access | Use HttpOnly/Secure/SameSite cookies; implement CSRF protection; sanitize all inputs |
| Insufficient test coverage | MEDIUM - Bugs in production | Enforce 90% coverage requirement; automated CI checks; comprehensive integration tests |
| Permission bypass vulnerabilities | CRITICAL - Data exposure | Backend permission enforcement; security code reviews; penetration testing |
| Database connection pool exhaustion | MEDIUM - Service unavailable | Configure connection limits; implement connection pooling; monitor connection usage |
