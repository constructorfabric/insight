# PRD — Git Stats Dashboard Frontend

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
   - [Authentication & Session Management](#51-authentication--session-management)
   - [Navigation & Page Management](#52-navigation--page-management)
   - [Filter System](#53-filter-system)
   - [Dashboard Page](#54-dashboard-page)
   - [Commits Table & Investigation](#55-commits-table--investigation)
   - [AI Adoption Page](#56-ai-adoption-page)
   - [Users Page](#57-users-page)
   - [Pull Requests Page](#58-pull-requests-page)
   - [Bitbucket Page](#59-bitbucket-page)
   - [Compliance Page](#510-compliance-page)
   - [Admin Features](#511-admin-features)
   - [Data Loading & Performance](#512-data-loading--performance)
   - [Analytics & Monitoring](#513-analytics--monitoring)
   - [User Experience](#514-user-experience)
6. [Non-Functional Requirements](#6-non-functional-requirements)
   - [NFR Inclusions](#61-nfr-inclusions)
   - [Explicitly Not Applicable Requirements](#62-explicitly-not-applicable-requirements)
7. [Public Library Interfaces](#7-public-library-interfaces)
   - [Public API Surface](#71-public-api-surface)
   - [External Integration Contracts](#72-external-integration-contracts)
8. [Use Cases](#8-use-cases)
   - [Authentication & Session Management](#81-authentication--session-management)
   - [Daily Dashboard Check-in](#82-daily-dashboard-check-in)
   - [Weekly Deep-Dive Analysis](#83-weekly-deep-dive-analysis)
   - [AI Adoption Systematic Review](#84-ai-adoption-systematic-review)
   - [Commits Table Investigation](#85-commits-table-investigation)
   - [Individual Contributor Performance Review](#86-individual-contributor-performance-review)
   - [Bitbucket PR Review Session](#87-bitbucket-pr-review-session)
   - [Filter Preset Management](#88-filter-preset-management)
   - [Mobile Weekend Planning](#89-mobile-weekend-planning)
   - [Admin Manages User Permissions](#810-admin-manages-user-permissions)
9. [Acceptance Criteria](#9-acceptance-criteria)
10. [Dependencies](#10-dependencies)
11. [Assumptions](#11-assumptions)
12. [Risks](#12-risks)

## 1. Overview

### 1.1 Purpose

The Git Stats Dashboard Frontend is a single-page application (SPA) that provides engineering managers, team leads, and developers with comprehensive analytics and visualizations of git activity, AI tool adoption, pull request metrics, and team productivity. It serves as the primary user interface for accessing analytics data through a secure backend API, enabling data-driven decision making and team performance monitoring.

### 1.2 Background / Problem Statement

Engineering organizations need visibility into developer productivity, code quality, AI tool adoption, and team collaboration patterns. Without a centralized analytics dashboard, managers must manually aggregate data from multiple sources (Git, Bitbucket, GitHub, Panopticum), leading to time-consuming reporting workflows and delayed insights.

The Git Stats Dashboard addresses these challenges by providing:
- Real-time analytics dashboards with interactive visualizations
- Comprehensive filtering system for multi-dimensional data analysis
- AI adoption tracking and enablement monitoring
- Pull request velocity and code review metrics
- Individual contributor performance tracking
- Compliance auditing and quality monitoring
- Mobile-responsive design for on-the-go access

The dashboard serves as the universal entry point for engineering analytics, with users returning frequently for daily check-ins, weekly deep-dives, and executive reporting.

### 1.3 Goals (Business Outcomes)

- Enable daily team monitoring with sub-3-second dashboard load times (p95)
- Support 100+ concurrent users with responsive UI (p95 < 500ms interaction time)
- Reduce weekly reporting time by 60-90 minutes per manager through automated visualizations
- Achieve 90%+ user adoption within engineering organization
- Provide mobile-responsive access for weekend/on-the-go monitoring
- Enable self-service analytics reducing ad-hoc data requests by 70%

### 1.4 Glossary

| Term | Definition |
|------|------------|
| SPA | Single-Page Application - client-side rendered web application |
| LOC | Lines of Code - measure of code volume (added + deleted + modified) |
| AI LOC | Lines of code generated or assisted by AI tools (Copilot, Windsurf, Claude) |
| Contributors Breakdown | Primary dashboard table showing per-user productivity metrics |
| Filter Preset | Saved combination of filter settings for quick workflow access |
| Progressive Loading | Phased data loading strategy (commits → PRs → AI metrics) |
| TrackedChart | Analytics event tracking wrapper for user interaction monitoring |

## 2. Actors

### 2.1 Human Actors

#### Engineering Manager

**ID**: `cpt-gitstats-actor-eng-manager`

**Role**: Primary user responsible for team performance monitoring, weekly reporting, and individual contributor management. Accesses dashboard daily for team pulse checks and weekly for deep-dive analysis.

**Needs**:
- Daily dashboard check-in (2-5 minutes)
- Weekly deep-dive analysis (30-60 minutes)
- AI adoption tracking and enablement
- Individual contributor performance reviews
- Automated report generation
- Mobile access for weekend planning

#### Team Lead

**ID**: `cpt-gitstats-actor-team-lead`

**Role**: Technical lead monitoring team productivity, code quality, and AI tool adoption. Focuses on commit-level analysis and pull request velocity.

**Needs**:
- Commits table investigation workflows
- Pull request review metrics
- Language-specific productivity analysis
- Repository health monitoring
- Compliance violation tracking

#### VP/Director of Engineering

**ID**: `cpt-gitstats-actor-executive`

**Role**: Executive requiring high-level organizational metrics, department comparisons, and strategic insights for leadership reporting.

**Needs**:
- Executive summary views
- Department comparison dashboards
- AI tool ROI analysis
- Quarterly trend reporting
- Mobile-optimized access

#### Developer/Individual Contributor

**ID**: `cpt-gitstats-actor-developer`

**Role**: Individual developer tracking personal productivity, AI tool usage, and comparing performance with team averages.

**Needs**:
- Personal performance dashboard
- AI tool usage statistics
- Commit history and language breakdown
- Repository contribution tracking

#### System Administrator

**ID**: `cpt-gitstats-actor-admin`

**Role**: Administrator managing user permissions, page visibility, repository tags, author aliases, and system configuration.

**Needs**:
- User management interface
- Permission configuration
- Repository tag management
- Author alias administration
- Dependency exclusion configuration
- Analytics event monitoring

### 2.2 System Actors

#### Backend API

**ID**: `cpt-gitstats-actor-backend-api`

**Role**: Backend REST API providing authentication, authorization, and data proxy services to analytics database.

**Needs**:
- Receive authenticated API requests
- Return paginated analytics data
- Provide user permissions and profile information
- Handle filter parameter validation

#### Analytics Database

**ID**: `cpt-gitstats-actor-analytics-database`

**Role**: Analytics database storing git commit data, pull request metrics, AI tool usage, and organizational metadata. (accessed via backend proxy).

**Needs**:
- Execute analytics queries via backend
- Return aggregated metrics
- Support complex filtering and grouping

#### Analytics Tracking Service

**ID**: `cpt-gitstats-actor-analytics`

**Role**: Client-side analytics system tracking user interactions, page views, and chart engagement for usage analysis.

**Needs**:
- Receive interaction events
- Track page navigation
- Monitor chart interactions
- Support behavioral analysis

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires modern web browser with current standards support
- Client-side scripting enabled
- Minimum screen resolution 1280x720 (responsive down to 375px mobile)
- Network connectivity to backend API (HTTPS required in production)
- Local storage support for filter presets and user settings
- Session storage for authentication tokens
- WebSocket support for real-time updates (future consideration)

## 4. Scope

### 4.1 In Scope

- Single-page application with client-side routing (hash-based)
- Comprehensive filtering system with 30+ filter options
- 15+ page views (Dashboard, AI Adoption, Users, Pull Requests, Bitbucket, etc.)
- Interactive data visualizations
- Progressive data loading strategy (commits → PRs → AI metrics)
- Filter preset management (save/load/delete workflows)
- User authentication and session management
- Permission-based page and chart visibility
- Mobile-responsive layouts (375px-1920px)
- Dark/light theme support
- Admin interface for user and permission management
- Analytics event tracking (page views, chart interactions)
- Health check and status endpoints
- Automated testing with 90%+ code coverage
- FAQ and What's New modals
- Code splitting and lazy loading for performance
- URL-based filter persistence
- Automated testing with 90%+ coverage

### 4.2 Out of Scope

- Server-side rendering (SSR)
- Native mobile applications
- Offline mode and service workers
- Real-time collaborative features
- Data export to Excel/CSV (delegated to backend)
- Email notifications and alerts
- Custom dashboard builder (drag-and-drop widgets)
- Advanced data visualization (heatmaps, network graphs)
- Multi-language UI translation (English only for initial release)
- RTL language support
- Cultural customization
- Full WCAG 2.1 AA compliance (baseline accessibility provided, full compliance future release)

## 5. Functional Requirements

### 5.1 Authentication & Session Management

#### SSO Login Flow

- [ ] `p1` - **ID**: `cpt-gitstats-fr-sso-login-ui`

The system **MUST** provide SSO login button that redirects to backend OIDC authentication endpoint and handles callback with session establishment.

**Rationale**: Enterprise security requires centralized SSO authentication with seamless user experience.

**Actors**: `cpt-gitstats-actor-eng-manager`, `cpt-gitstats-actor-backend-api`

#### Local Login Form

- [ ] `p2` - **ID**: `cpt-gitstats-fr-local-login-ui`

The system **MUST** provide local username/password login form for development and emergency access.

**Rationale**: Development environments and emergency scenarios require fallback authentication.

**Actors**: `cpt-gitstats-actor-developer`

#### Session Persistence

- [ ] `p1` - **ID**: `cpt-gitstats-fr-session-persistence`

The system **MUST** persist authentication session across page refreshes and browser tabs using secure cookies.

**Rationale**: Users expect continuous session without re-authentication on every page load.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Authentication Status Check

- [ ] `p1` - **ID**: `cpt-gitstats-fr-auth-status-check`

The system **MUST** check authentication status on application load and redirect to login if unauthenticated.

**Rationale**: Protected application requires authentication verification before rendering content.

**Actors**: `cpt-gitstats-actor-backend-api`

#### Logout Functionality

- [ ] `p1` - **ID**: `cpt-gitstats-fr-logout`

The system **MUST** provide logout button that clears session and redirects to login page.

**Rationale**: Users need ability to securely end session and switch accounts.

**Actors**: `cpt-gitstats-actor-eng-manager`

### 5.2 Navigation & Page Management

#### Hash-Based Routing

- [ ] `p1` - **ID**: `cpt-gitstats-fr-hash-routing`

The system **MUST** implement hash-based client-side routing with browser back/forward support and URL persistence.

**Rationale**: SPA requires client-side navigation without full page reloads while supporting browser history.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Page Permission Enforcement

- [ ] `p1` - **ID**: `cpt-gitstats-fr-page-permissions`

The system **MUST** enforce page-level permissions by hiding restricted pages from navigation and showing access denied message on direct access.

**Rationale**: User permissions must be enforced in UI to prevent unauthorized page access.

**Actors**: `cpt-gitstats-actor-eng-manager`, `cpt-gitstats-actor-backend-api`

#### Lazy Loading

- [ ] `p1` - **ID**: `cpt-gitstats-fr-lazy-loading`

The system **MUST** implement code splitting and lazy loading for all page views to minimize initial bundle size.

**Rationale**: Performance optimization requires loading only necessary code for current page.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Mobile Navigation Drawer

- [ ] `p2` - **ID**: `cpt-gitstats-fr-mobile-nav`

The system **MUST** provide mobile-responsive navigation drawer with hamburger menu for screens < 768px.

**Rationale**: Mobile users need accessible navigation on small screens.

**Actors**: `cpt-gitstats-actor-executive`

### 5.3 Filter System

#### Global Filter Context

- [ ] `p1` - **ID**: `cpt-gitstats-fr-filter-context`

The system **MUST** provide global FilterContext managing 30+ filter options with state persistence across page navigation.

**Rationale**: Consistent filtering across all pages enables coherent multi-page analysis workflows.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Filter Bar Component

- [ ] `p1` - **ID**: `cpt-gitstats-fr-filter-bar`

The system **MUST** provide FilterBar component with date range, users, roles, units, teams, organizations, repositories, languages, and AI tool filters.

**Rationale**: Comprehensive filtering enables multi-dimensional data analysis.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### URL Filter Persistence

- [ ] `p1` - **ID**: `cpt-gitstats-fr-url-filters`

The system **MUST** encode active filters in URL query parameters for shareable links and browser back/forward support.

**Rationale**: Users need to share filtered views and restore filter state from browser history.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Filter Presets

- [ ] `p2` - **ID**: `cpt-gitstats-fr-filter-presets`

The system **MUST** allow users to save, load, and delete filter preset combinations for quick workflow access.

**Rationale**: Repetitive workflows (daily check-in, weekly review, AI adoption) require quick filter restoration.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Default Filters

- [ ] `p1` - **ID**: `cpt-gitstats-fr-default-filters`

The system **MUST** apply sensible default filters (14 days, Technology org, exclude large commits, exclude merge commits, exclude system accounts).

**Rationale**: New users need productive defaults without manual configuration.

**Actors**: `cpt-gitstats-actor-eng-manager`

### 5.4 Dashboard Page

#### Key Metrics Cards

- [ ] `p1` - **ID**: `cpt-gitstats-fr-dashboard-metrics`

The system **MUST** display four key metric cards: Total Commits, Lines of Code, AI Involvement %, Active/Total Users.

**Rationale**: At-a-glance metrics provide quick team pulse for daily check-ins.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### AI LOC Analysis Chart

- [ ] `p1` - **ID**: `cpt-gitstats-fr-ai-loc-chart`

The system **MUST** provide interactive time-series chart showing AI-assisted LOC trends with configurable granularity (Day/Week/Month/Quarter), moving average, tool filtering, and user comparison.

**Rationale**: AI adoption tracking is strategic priority for engineering organizations.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Contributors Breakdown Table

- [ ] `p1` - **ID**: `cpt-gitstats-fr-contributors-table`

The system **MUST** provide sortable Contributors Breakdown table showing per-user metrics (commits, LOC, AI LOC, AI %, top languages).

**Rationale**: Primary tool for identifying outliers and team performance.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Department Breakdown Table

- [ ] `p2` - **ID**: `cpt-gitstats-fr-department-table`

The system **MUST** provide Department Breakdown table with aggregated metrics by organizational unit.

**Rationale**: Executives need department-level comparisons for strategic planning.

**Actors**: `cpt-gitstats-actor-executive`

#### Repository Metrics Table

- [ ] `p2` - **ID**: `cpt-gitstats-fr-repository-table`

The system **MUST** provide Repository Metrics table showing per-repository activity and contributor counts.

**Rationale**: Repository health monitoring identifies low-activity or orphaned repositories.

**Actors**: `cpt-gitstats-actor-team-lead`

### 5.5 Commits Table & Investigation

#### Commits Table Component

- [ ] `p1` - **ID**: `cpt-gitstats-fr-commits-table`

The system **MUST** provide paginated Commits Table with sorting, filtering, and drill-down to commit details.

**Rationale**: Primary investigation tool for detailed commit analysis.

**Actors**: `cpt-gitstats-actor-team-lead`

#### Commit Details Modal

- [ ] `p1` - **ID**: `cpt-gitstats-fr-commit-details`

The system **MUST** provide Commit Details Modal showing commit metadata, file changes, AI tool usage, and related pull requests.

**Rationale**: Deep investigation requires comprehensive commit-level information.

**Actors**: `cpt-gitstats-actor-team-lead`

#### Commits Filtering

- [ ] `p1` - **ID**: `cpt-gitstats-fr-commits-filtering`

The system **MUST** support commit-specific filtering by user, repository, date range, AI tool, and change type.

**Rationale**: Targeted investigation requires granular filtering beyond global filters.

**Actors**: `cpt-gitstats-actor-team-lead`

### 5.6 AI Adoption Page

#### AI Adoption Dashboard

- [ ] `p1` - **ID**: `cpt-gitstats-fr-ai-dashboard`

The system **MUST** provide comprehensive AI Adoption page with Copilot charts, Windsurf credits, AI users table, and adoption breakdown.

**Rationale**: Systematic AI metrics review enables adoption tracking and enablement.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### AI Users Table

- [ ] `p1` - **ID**: `cpt-gitstats-fr-ai-users-table`

The system **MUST** provide AI Users Table showing per-user AI tool adoption, credits consumed, and lines generated.

**Rationale**: Identify non-adopters for coaching and track enablement progress.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Windsurf/Copilot Credits Charts

- [ ] `p1` - **ID**: `cpt-gitstats-fr-ai-credits-charts`

The system **MUST** provide separate charts for Windsurf and Copilot credit consumption and license utilization.

**Rationale**: ROI tracking requires visibility into license usage and credit burn rates.

**Actors**: `cpt-gitstats-actor-executive`

### 5.7 Users Page

#### User List Table

- [ ] `p1` - **ID**: `cpt-gitstats-fr-user-list`

The system **MUST** provide User List table with performance metrics, organizational info, and drill-down to user details.

**Rationale**: Individual contributor tracking for performance reviews and 1-on-1 preparation.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### User Details Modal

- [ ] `p1` - **ID**: `cpt-gitstats-fr-user-details`

The system **MUST** provide User Details Modal with commit history, language breakdown, repository contributions, AI usage, and team comparison.

**Rationale**: Comprehensive individual performance review for 1-on-1 preparation.

**Actors**: `cpt-gitstats-actor-eng-manager`

### 5.8 Pull Requests Page

#### PR List Table

- [ ] `p1` - **ID**: `cpt-gitstats-fr-pr-list`

The system **MUST** provide Pull Request list table with status, review time, reviewer info, and drill-down to PR details.

**Rationale**: PR velocity monitoring is critical for team productivity.

**Actors**: `cpt-gitstats-actor-team-lead`

#### PR Details Modal

- [ ] `p1` - **ID**: `cpt-gitstats-fr-pr-details`

The system **MUST** provide PR Details Modal showing commits, reviewers, comments, and approval timeline.

**Rationale**: Detailed PR investigation for bottleneck identification.

**Actors**: `cpt-gitstats-actor-team-lead`

### 5.9 Bitbucket Page

#### Bitbucket Metrics Dashboard

- [ ] `p1` - **ID**: `cpt-gitstats-fr-bitbucket-dashboard`

The system **MUST** provide Bitbucket page with repository metrics, velocity trends, review time, reviewer performance, and tech debt charts.

**Rationale**: Systematic review of Bitbucket metrics enables PR velocity monitoring.

**Actors**: `cpt-gitstats-actor-eng-manager`

### 5.10 Compliance Page

#### 4 Eyes Compliance Dashboard

- [ ] `p2` - **ID**: `cpt-gitstats-fr-compliance-dashboard`

The system **MUST** provide 4 Eyes Compliance page showing violations, affected commits, and remediation tracking.

**Rationale**: Regular compliance auditing ensures code review policy adherence.

**Actors**: `cpt-gitstats-actor-admin`

### 5.11 Admin Features

#### User Management Interface

- [ ] `p1` - **ID**: `cpt-gitstats-fr-user-management`

The system **MUST** provide admin interface for creating users, assigning roles, and configuring permissions.

**Rationale**: Administrators need UI for user provisioning and permission management.

**Actors**: `cpt-gitstats-actor-admin`

#### Repository Tags Management

- [ ] `p2` - **ID**: `cpt-gitstats-fr-repo-tags`

The system **MUST** provide interface for creating, assigning, and managing repository tags for filtering.

**Rationale**: Repository categorization enables targeted analysis and reporting.

**Actors**: `cpt-gitstats-actor-admin`

#### Author Aliases Management

- [ ] `p2` - **ID**: `cpt-gitstats-fr-author-aliases`

The system **MUST** provide interface for mapping git author names/emails to canonical user identities.

**Rationale**: Data normalization requires alias management for accurate attribution.

**Actors**: `cpt-gitstats-actor-admin`

#### Dependency Exclusions Configuration

- [ ] `p2` - **ID**: `cpt-gitstats-fr-dependency-exclusions`

The system **MUST** provide interface for configuring dependency file exclusion patterns.

**Rationale**: Accurate productivity metrics require excluding auto-generated dependency files.

**Actors**: `cpt-gitstats-actor-admin`

### 5.12 Data Loading & Performance

#### Progressive Data Loading

- [ ] `p1` - **ID**: `cpt-gitstats-fr-progressive-loading`

The system **MUST** implement progressive loading strategy: Phase 1 (commits), Phase 2 (PRs), Phase 3 (AI metrics) with loading indicators.

**Rationale**: Perceived performance requires showing initial data quickly while loading additional metrics.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Loading Overlays

- [ ] `p1` - **ID**: `cpt-gitstats-fr-loading-overlays`

The system **MUST** display loading overlays on charts during data fetch with current action messages.

**Rationale**: User feedback during data loading prevents perceived freezing.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Data Caching

- [ ] `p1` - **ID**: `cpt-gitstats-fr-data-caching`

The system **MUST** cache filtered data in FilteredDataContext to prevent redundant calculations on filter changes.

**Rationale**: Performance optimization requires avoiding re-computation of expensive aggregations.

**Actors**: `cpt-gitstats-actor-eng-manager`

### 5.13 Analytics & Monitoring

#### Page View Tracking

- [ ] `p1` - **ID**: `cpt-gitstats-fr-page-tracking`

The system **MUST** track page views with user ID, page name, and timestamp.

**Rationale**: Usage analytics enable behavioral analysis and feature prioritization.

**Actors**: `cpt-gitstats-actor-analytics`

#### Chart Interaction Tracking

- [ ] `p1` - **ID**: `cpt-gitstats-fr-chart-tracking`

The system **MUST** track chart interactions (clicks, filters, sorts) using TrackedChart wrapper.

**Rationale**: Interaction analytics identify most-used features and optimization opportunities.

**Actors**: `cpt-gitstats-actor-analytics`

### 5.14 User Experience

#### Theme Support

- [ ] `p2` - **ID**: `cpt-gitstats-fr-theme-support`

The system **MUST** provide dark/light theme toggle with user preference persistence.

**Rationale**: User comfort and accessibility require theme customization.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Mobile Responsiveness

- [ ] `p2` - **ID**: `cpt-gitstats-fr-mobile-responsive`

The system **MUST** provide mobile-responsive layouts for screens 375px-768px with touch-optimized interactions.

**Rationale**: Mobile access enables weekend planning and on-the-go monitoring.

**Actors**: `cpt-gitstats-actor-executive`

#### FAQ Modal

- [ ] `p2` - **ID**: `cpt-gitstats-fr-faq-modal`

The system **MUST** provide FAQ modal with common questions, filter explanations, and feature documentation.

**Rationale**: Self-service help reduces support burden and improves user onboarding.

**Actors**: `cpt-gitstats-actor-developer`

#### What's New Modal

- [ ] `p2` - **ID**: `cpt-gitstats-fr-whats-new`

The system **MUST** provide What's New modal showing recent features and updates.

**Rationale**: Feature discovery and user engagement require update notifications.

**Actors**: `cpt-gitstats-actor-eng-manager`

#### Accessibility Baseline

- [ ] `p2` - **ID**: `cpt-gitstats-fr-accessibility-baseline`

The system **MUST** provide baseline accessibility support:
- Keyboard navigation for all interactive elements (Tab, Enter, Escape)
- Focus indicators visible on all focusable elements
- Color contrast ratio ≥ 4.5:1 for normal text (WCAG AA minimum)
- Alt text for all informational images and charts
- Semantic HTML structure (headings, landmarks, lists)
- Screen reader compatibility for critical workflows (Dashboard, Commits Table)

**Rationale**: Baseline accessibility ensures usability for users with disabilities and reduces legal/compliance risk.

**Future**: Full WCAG 2.1 AA compliance planned for future release.

**Actors**: `cpt-gitstats-actor-eng-manager`, `cpt-gitstats-actor-developer`

#### Regional Format Support

- [ ] `p2` - **ID**: `cpt-gitstats-fr-regional-formats`

The system **MUST** support regional format preferences:
- Date format configurable (MM/DD/YYYY, DD/MM/YYYY, YYYY-MM-DD)
- Number format based on browser locale (1,000.00 vs 1.000,00)
- Timezone display for timestamps (UTC or user's local timezone)

**Rationale**: International users (40% Asia Pacific) require localized formats even with English UI.

**Actors**: `cpt-gitstats-actor-eng-manager`, `cpt-gitstats-actor-executive`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Dashboard Load Time

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-dashboard-load`

The system **MUST** load Dashboard page to interactive state within 3 seconds at p95 under normal load (100 concurrent users).

**Threshold**: p95 < 3s for initial page load, p95 < 1s for subsequent navigation

**Rationale**: User engagement requires fast initial load. Dashboard is universal entry point requiring fast navigation.

#### UI Interaction Responsiveness

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-ui-responsiveness`

The system **MUST** respond to user interactions (clicks, filters, sorts) within 500ms at p95.

**Threshold**: p95 < 500ms for UI interactions, p95 < 100ms for filter updates

**Rationale**: Responsive UI is critical for user satisfaction and workflow efficiency.

#### Chart Rendering Performance

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-chart-rendering`

The system **MUST** render charts with up to 1000 data points within 1 second.

**Threshold**: < 1s for chart rendering with 1000 points, < 2s for 5000 points

**Rationale**: Interactive charts require fast rendering for data exploration.

#### Bundle Size

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-bundle-size`

The system **MUST** maintain initial JavaScript bundle size < 500KB gzipped with code splitting for lazy-loaded pages.

**Threshold**: Initial bundle < 500KB gzipped, total bundle < 2MB

**Rationale**: Fast initial load requires minimal bundle size with progressive enhancement.

#### Memory Usage

- [ ] `p2` - **ID**: `cpt-gitstats-nfr-memory-usage`

The system **MUST** maintain browser memory usage < 200MB for typical workflows with proper cleanup on page navigation.

**Threshold**: < 200MB for normal usage, < 500MB for extended sessions

**Rationale**: Long-running sessions (30-60 minute deep-dives) require efficient memory management.

#### Browser Compatibility

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-browser-compat`

The system **MUST** support Chrome 90+, Firefox 88+, Safari 14+, Edge 90+ with consistent functionality.

**Threshold**: 100% feature parity across supported browsers

**Rationale**: Enterprise users access from various browsers requiring cross-browser compatibility.

#### Mobile Performance

- [ ] `p2` - **ID**: `cpt-gitstats-nfr-mobile-performance`

The system **MUST** provide responsive layouts for screens 375px-1920px with touch-optimized interactions.

**Threshold**: Usable on screens ≥ 375px width, optimized for 768px-1920px

**Rationale**: Weekend executive reviews require mobile access.

#### Test Coverage

- [ ] `p1` - **ID**: `cpt-gitstats-nfr-test-coverage`

The system **MUST** maintain automated test coverage ≥ 90% for components, hooks, and utilities.

**Threshold**: 90% code coverage measured by Jest

**Rationale**: High test coverage ensures reliability and enables safe refactoring.

#### Data Classification and Client-Side Storage

- [ ] `p2` - **ID**: `cpt-gitstats-nfr-data-classification`

The system **MUST** classify and handle client-side data according to sensitivity:

**Session Storage** (cleared on browser close):
- Authentication tokens (sensitive)
- Current filter state (non-sensitive)

**Local Storage** (persistent):
- Filter presets (non-sensitive)
- Theme preference (non-sensitive)
- User settings (non-sensitive)

**Memory Only** (never persisted):
- Analytics data from backend (potentially sensitive)
- User performance metrics (sensitive)

**Threshold**: Zero sensitive data persisted to local storage, all authentication tokens in session storage only

**Rationale**: Clear data classification ensures appropriate handling and prevents data leakage on shared computers.

**Retention**: Local storage data retained indefinitely until user clears browser data or deletes presets. No automatic cleanup required for non-sensitive data.

### 6.2 Explicitly Not Applicable Requirements

The following quality characteristics from ISO/IEC 25010:2023 and Constructor Studio PRD checklist are **intentionally not applicable** to this frontend PRD:

#### Safety (SAFE)

**Not applicable**: Pure information system with no physical interaction, medical devices, vehicles, or industrial control. No operations that could cause harm to people, property, or environment.

#### Regulatory Compliance (COMPL)

**Not applicable**: Internal enterprise tool. No direct PII processing (analytics tracking optional, no PII collection). Regulatory compliance (GDPR, HIPAA, SOX) delegated to backend API.

#### Operations (OPS - Deployment/Monitoring)

**Not applicable**: Frontend SPA deployment and monitoring delegated to infrastructure team. No deployment/monitoring requirements at PRD level.

#### Inclusivity (UX-PRD-005)

**Not applicable**: Internal tool with known, narrow user base (engineering managers, team leads). No diverse user populations requiring specialized inclusivity considerations beyond baseline accessibility.

#### Privacy by Design (SEC-PRD-005)

**Not applicable**: Frontend delegates all PII processing to backend. No direct personal data processing in client code.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### React Component Library

- [ ] `p2` - **ID**: `cpt-gitstats-interface-components`

**Type**: React component library

**Stability**: internal (not published)

**Description**: Reusable components (FilterBar, CommitsTable, StatCard, TrackedChart, etc.) for internal application use.

**Breaking Change Policy**: Internal components may change between versions without notice.

### 7.2 External Integration Contracts

#### Backend REST API Contract

- [ ] `p1` - **ID**: `cpt-gitstats-contract-backend-api`

**Direction**: required from client

**Protocol/Format**: HTTP/REST JSON API

**Compatibility**: Compatible with backend API v1 endpoints

**Key Endpoints**:
- `/api/status/` - Authentication status
- `/api/me/` - Current user info and permissions
- `/api/data/v1/dashboard/` - Dashboard metrics
- `/api/data/v1/commits/` - Commits data
- `/api/data/v1/pull-requests/` - Pull request data
- `/api/data/v1/ai-adoption/` - AI metrics
- `/api/users/` - User management

#### Analytics Tracking Contract

- [ ] `p2` - **ID**: `cpt-gitstats-contract-analytics`

**Direction**: provided by library

**Protocol/Format**: Custom event tracking API

**Compatibility**: Internal analytics schema

**Events**: page_view, chart_interaction, filter_change, modal_open

## 8. Use Cases

### 8.1 Authentication & Session Management

#### User Logs In via SSO

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-sso-login-ui`

**Actor**: `cpt-gitstats-actor-eng-manager`

**Preconditions**:
- User navigates to application URL
- User is not authenticated
- Backend SSO is operational

**Main Flow**:
1. Application loads and detects no authentication
2. LoginPage component renders with SSO button
3. User clicks "Login with SSO"
4. Frontend redirects to backend `/oidc/authenticate/`
5. Backend handles OIDC flow and redirects back
6. Frontend receives session cookie
7. AuthContext updates with user info
8. Application redirects to Dashboard

**Postconditions**:
- User is authenticated with active session
- User permissions loaded
- Dashboard renders with user data

### 8.2 Daily Dashboard Check-in

#### Manager Performs Daily Team Check

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-daily-checkin-ui`

**Actor**: `cpt-gitstats-actor-eng-manager`

**Frequency**: Daily (60% of sessions, 2-5 minutes)

**Preconditions**:
- User is authenticated
- Default filters applied (14 days, Technology org)

**Main Flow**:
1. User opens application (Dashboard loads by default)
2. ProgressiveDataProvider loads Phase 1 (commits data)
3. Key metrics cards render with loading state
4. Commits data loads, metrics cards update
5. Contributors Breakdown table renders
6. User scans table for outliers (high/low performers)
7. Phase 2 (PRs) and Phase 3 (AI metrics) load in background
8. User identifies team member with low activity
9. User clicks on contributor row
10. User Details Modal opens with comprehensive metrics

**Postconditions**:
- User has team pulse overview
- Outliers identified for follow-up
- Page view tracked in analytics

### 8.3 Weekly Deep-Dive Analysis

#### Manager Prepares Weekly Team Review

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-weekly-deepdive-ui`

**Actor**: `cpt-gitstats-actor-eng-manager`

**Frequency**: Weekly (30-60 minutes)

**Preconditions**:
- User is authenticated
- Weekly team meeting scheduled

**Main Flow**:
1. User opens Dashboard
2. User adjusts date range to last 7 days
3. User reviews Contributors Breakdown table
4. User sorts by commits descending to identify top performers
5. User clicks on multiple contributors to review details
6. User navigates to Commits page
7. User filters commits by specific team members
8. User investigates unusual commits in detail
9. User navigates to Bitbucket page
10. User reviews PR velocity and review time metrics
11. User navigates to AI Adoption page
12. User checks AI tool adoption progress
13. User navigates to Reports page for trend analysis
14. User takes screenshots for meeting presentation

**Postconditions**:
- Weekly team review prepared
- Key insights identified for discussion
- Screenshots captured for presentation

### 8.4 AI Adoption Systematic Review

#### Manager Tracks AI Tool Adoption

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-ai-adoption-review-ui`

**Actor**: `cpt-gitstats-actor-eng-manager`

**Frequency**: 2-5x per week (5-10 minutes)

**Preconditions**:
- User is authenticated
- User has AI Adoption page permission

**Main Flow**:
1. User navigates to AI Adoption page
2. Page loads with all AI metrics charts
3. User reviews Copilot charts (usage trends)
4. User reviews Windsurf credits chart (license utilization)
5. User reviews AI Users Table (per-user adoption)
6. User sorts table by AI LOC descending
7. User identifies non-adopters (0 AI LOC)
8. User reviews AI Active Users chart
9. User checks Missing Panopticum Users section
10. User notes action items for enablement

**Postconditions**:
- AI adoption status assessed
- Non-adopters identified for coaching
- License utilization tracked

### 8.5 Commits Table Investigation

#### Team Lead Investigates Code Quality Issue

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-commits-investigation-ui`

**Actor**: `cpt-gitstats-actor-team-lead`

**Frequency**: Daily to weekly (10-30 minutes)

**Preconditions**:
- User is authenticated
- User identified outlier in Dashboard

**Main Flow**:
1. User identifies contributor with unusually high LOC in Dashboard
2. User clicks on contributor row
3. User Details Modal opens
4. User clicks "View Commits" button
5. Commits Table renders filtered by user
6. User sorts by LOC descending
7. User identifies commit with 15,000 LOC
8. User clicks on commit row
9. Commit Details Modal opens
10. User reviews file changes and metadata
11. User checks if commit is auto-generated or legitimate
12. User closes modal and continues investigation

**Postconditions**:
- Code quality issue investigated
- Large commit identified and validated
- Follow-up action determined

### 8.6 Individual Contributor Performance Review

#### Manager Prepares 1-on-1 Meeting

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-individual-review-ui`

**Actor**: `cpt-gitstats-actor-eng-manager`

**Frequency**: 2-5x per week (15-30 minutes)

**Preconditions**:
- User is authenticated
- 1-on-1 meeting scheduled

**Main Flow**:
1. User navigates to Users page
2. User List table renders with performance metrics
3. User locates direct report in table
4. User clicks on user row
5. User Details Modal opens with comprehensive metrics
6. User reviews commit history timeline
7. User reviews language breakdown (TypeScript 60%, Python 30%)
8. User reviews repository contributions
9. User reviews AI tool usage (Copilot 45%, Windsurf 10%)
10. User compares metrics with team average
11. User notes discussion points for 1-on-1
12. User closes modal

**Postconditions**:
- Individual performance assessment completed
- 1-on-1 meeting preparation ready
- Discussion points identified

### 8.7 Bitbucket PR Review Session

#### Manager Reviews PR Velocity Metrics

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-pr-review-ui`

**Actor**: `cpt-gitstats-actor-eng-manager`

**Frequency**: 1-3x per week (10-20 minutes)

**Preconditions**:
- User is authenticated
- User has Bitbucket page permission

**Main Flow**:
1. User navigates to Bitbucket page
2. Repository Metrics table renders
3. User reviews velocity trend chart
4. User identifies velocity drop in last week
5. User reviews review time metrics chart
6. User identifies increased review time (3 days avg)
7. User reviews reviewer performance chart
8. User identifies bottleneck reviewer
9. User reviews tech debt chart
10. User notes action items for team discussion

**Postconditions**:
- PR velocity status assessed
- Bottlenecks identified
- Action items for team improvement

### 8.8 Filter Preset Management

#### User Saves Daily Check-in Workflow

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-filter-preset-ui`

**Actor**: `cpt-gitstats-actor-eng-manager`

**Preconditions**:
- User is authenticated
- User has configured filters for daily workflow

**Main Flow**:
1. User configures filters (7 days, specific team, exclude large commits)
2. User clicks "Save Preset" button
3. Save Preset modal opens
4. User enters preset name "Daily Team Check"
5. User clicks "Save"
6. Preset saved to local storage
7. Preset appears in preset dropdown
8. Next day, user opens application
9. User selects "Daily Team Check" from preset dropdown
10. Filters instantly applied
11. Dashboard updates with filtered data

**Postconditions**:
- Filter preset saved for reuse
- Daily workflow streamlined
- Time saved on filter configuration

### 8.9 Mobile Weekend Planning

#### Executive Reviews Metrics on Mobile

- [ ] `p2` - **ID**: `cpt-gitstats-usecase-mobile-weekend-ui`

**Actor**: `cpt-gitstats-actor-executive`

**Frequency**: Weekends (15-30 minutes)

**Preconditions**:
- User is authenticated
- User accessing from mobile device (< 768px)

**Main Flow**:
1. User opens application on mobile device
2. Mobile navigation drawer renders
3. User taps hamburger menu
4. Navigation drawer slides in
5. User taps "Dashboard"
6. Dashboard renders in mobile layout
7. Key metrics cards stack vertically
8. User scrolls to view all metrics
9. User taps on Contributors Breakdown
10. Table renders with horizontal scroll
11. User reviews high-level metrics
12. User closes application

**Postconditions**:
- High-level metrics reviewed
- Weekly planning insights obtained
- No desktop required for weekend check

### 8.10 Admin Manages User Permissions

#### Admin Configures Page Visibility

- [ ] `p1` - **ID**: `cpt-gitstats-usecase-admin-permissions-ui`

**Actor**: `cpt-gitstats-actor-admin`

**Preconditions**:
- User is authenticated with admin role
- Target user exists in system

**Main Flow**:
1. Admin navigates to User Management page
2. User list renders with all users
3. Admin searches for target user
4. Admin clicks on user row
5. User edit modal opens
6. Admin navigates to Permissions tab
7. Available pages/charts list renders
8. Admin unchecks "Debug" page
9. Admin unchecks "Compliance" chart
10. Admin clicks "Save"
11. Backend updates user permissions
12. Success notification displays

**Postconditions**:
- User permissions updated
- Debug page hidden from user navigation
- Compliance chart hidden from dashboards

## 9. Acceptance Criteria

- [ ] SSO login successfully authenticates users and establishes session
- [ ] Local login provides fallback authentication for development
- [ ] Dashboard loads to interactive state within 3 seconds (p95)
- [ ] All pages enforce permission-based visibility
- [ ] Filter system supports 30+ filter options with URL persistence
- [ ] Filter presets can be saved, loaded, and deleted
- [ ] Progressive loading displays commits data within 2 seconds
- [ ] Contributors Breakdown table supports sorting and drill-down
- [ ] Commits Table provides pagination and detailed investigation
- [ ] User Details Modal shows comprehensive individual metrics
- [ ] AI Adoption page displays all AI tool metrics
- [ ] Bitbucket page shows PR velocity and review metrics
- [ ] Mobile responsive layouts work on screens ≥ 375px
- [ ] Analytics tracking captures page views and chart interactions
- [ ] Test coverage exceeds 90% for all components
- [ ] Application supports 100+ concurrent users
- [ ] Theme toggle persists user preference
- [ ] Code splitting reduces initial bundle to < 500KB gzipped

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| UI Framework | Component-based UI framework for SPA development | p1 |
| Chart Visualization Library | Interactive charting library for data visualization | p1 |
| CSS Framework | Styling framework for responsive design | p1 |
| Icon Library | Comprehensive icon set for UI elements | p1 |
| Backend REST API | Authentication and data proxy services | p1 |
| Modern Web Browser | Current standards-compliant web browser | p1 |
| Type System | Static type checking for code quality | p1 |
| Build Tooling | Development server and production build system | p1 |
| Testing Framework | Unit and integration testing utilities | p1 |
| Utility Libraries | Performance optimization utilities (debounce, throttle) | p2 |

## 11. Assumptions

- Backend API is accessible and operational
- Users have modern web browsers with client-side scripting enabled
- Network connectivity is reliable for API requests
- Local storage is available for filter presets and settings
- Session cookies are supported and not blocked
- HTTPS is used in production for secure communication
- Analytics database is pre-populated with data
- User permissions are managed by backend and enforced in UI
- Analytics tracking is optional and can be disabled
- Mobile users accept horizontal scrolling for large tables

### Client-Side Data Lifecycle

**Filter Presets**:
- Retained indefinitely in browser local storage
- User can delete individual presets via UI
- User can clear all presets via browser settings
- No automatic expiration

**User Settings**:
- Retained indefinitely in browser local storage
- User can reset to defaults via UI
- Cleared when user clears browser data

**Session Data**:
- Cleared automatically on browser close
- No persistence beyond session

**User Control**: Settings page provides "Clear All Local Data" button to reset application state

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Backend API downtime | HIGH - Application unusable | Implement retry logic, show meaningful error messages, cache last successful data |
| Large dataset performance | HIGH - Slow rendering, browser freeze | Implement pagination, virtualization, progressive loading, data sampling |
| Browser compatibility issues | MEDIUM - Inconsistent UX | Comprehensive cross-browser testing, polyfills for older browsers |
| Memory leaks in long sessions | MEDIUM - Browser crash | Proper cleanup in component lifecycle, memory profiling, session timeout |
| Bundle size growth | MEDIUM - Slow initial load | Code splitting, lazy loading, tree shaking, bundle analysis |
| Filter complexity overwhelming users | MEDIUM - Poor UX | Default filters, filter presets, guided workflows, FAQ documentation |
| Mobile performance degradation | MEDIUM - Poor mobile UX | Mobile-optimized layouts, reduced data on mobile, progressive enhancement |
| Analytics tracking privacy concerns | LOW - User trust issues | Make tracking optional, transparent privacy policy, no PII collection |
