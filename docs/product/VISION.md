# 1. Constructor Insight Vision

Constructor Insight is an AI-assisted intelligence product that helps organizations understand and improve how work is performed across roles, teams, systems, and cost centers.

It does not stop at reporting activity. Insight observes work from existing systems, uses AI and analytical rules to find problems in output, quality, cost, or flow, recommends concrete improvement actions, and later validates whether those actions improved the measured system.

A recommendation in Insight is a structured improvement object, not a generic comment or dashboard annotation. For every recommendation, Insight shows:

* the observed problem;
* the affected role, team, process, system, or cost area;
* the evidence and confidence behind the diagnosis;
* the recommended action;
* the owner who can act on it;
* the expected metric movement;
* the follow-up measurement window used to validate whether the action helped.

⠀
Insight supports multiple deployment models: Constructor-hosted cloud, customer cloud, private cloud, and customer-operated installation. In every model, customers configure their roles, activities, integrations, metrics, access rules, and localization.

Operational responsibility depends on the selected deployment model: Constructor operates the hosted service, while customer-operated deployments can be downloaded, installed, configured, updated, and upgraded by the customer. Customer data remains under customer control, and Insight does not require Constructor to have default access to customer data in order to operate.

## 2\. Open Design

Constructor Insight is tool-agnostic, source-agnostic, model-agnostic, and role-model configurable. It does not require an organization to standardize on one toolchain, one AI model, one methodology, or one predefined role taxonomy before using the product.

Open design means Insight is explainable and auditable. Customers can see what data Insight reads, how metrics are calculated, how AI-assisted analysis is used, and why a diagnosis or recommendation was produced.

Metrics, diagnoses, and recommendations are traceable to source evidence, with confidence and limitations shown where relevant.

Organizations retain ownership and control of their people, work, cost, and operational data while gaining a shared intelligence layer over how work is planned, built, sold, supported, and operated. Insight reads from the systems a company already uses and adapts to the customer’s operating model, localization, and governance rules.


## 3\. Governance

Governance is part of the product promise. Insight uses only the data required for the customer’s configured improvement workflows: measurement, diagnosis, recommendation, and validation. It works with observed work signals, not private opinions or self-reported productivity.

Access to people-level data is role-based and policy-controlled. Insight is not designed to default to individual stack ranking or unexplained productivity scores.

When evidence is incomplete, Insight shows the gap instead of hiding it. Recommendations, diagnoses, and suggestions are presented with confidence and limitations, so customers know whether they are seeing a strong finding, a directional signal, or an instrumentation problem.

## 4\. Why Insight

Organizations already have dashboards, activity reports, AI usage logs, engineering metrics, CRM reports, support analytics, cloud cost reports, and finance summaries. The problem is not the absence of numbers. The problem is that these numbers rarely explain what should be improved, who can improve it, and whether the improvement worked.

Insight exists to help organizations improve how work is performed. It finds problems in flow, output, quality, cost, AI usage, and role execution; explains why they matter; recommends actions; assigns the action to the role or owner who can act on it; and later validates whether the action changed the measured system.

Insight is different because it:

* covers the whole organization, not only engineering;
* connects people, roles, teams, work, systems, quality, cost, AI usage, and outcomes;
* follows work across its lifecycle — from intent through delivery, operation, support, and post-release impact;
* detects bottlenecks, waste, quality risks, cost anomalies, role mismatches, and weak evidence areas;
* uses AI to help analyze patterns, explain likely causes, generate recommendations, and summarize evidence;
* compares change over time, including before and after a process, tooling, staffing, or AI adoption change;
* moves from measurement to diagnosis, recommendation, action ownership, and validation;
* keeps conclusions explainable through evidence, confidence, and stated limitations.

⠀
The goal is not to create another dashboard layer. The goal is to give leaders and teams an improvement system: find what is not working, understand why, decide what to do, and check whether the action helped.

## 5\. How Insight Uses AI to Improve Work

Insight uses AI as part of the product intelligence layer, not only as a thing being measured.

AI helps Insight turn connected evidence into useful improvement actions. It does not replace the evidence model, metric definitions, lineage, governance rules, or human decision-making. Instead, AI helps analyze patterns, explain likely causes, summarize evidence, generate draft recommendations, and identify what evidence is missing before a stronger claim can be made.

Insight uses AI to:

* **Find problems** — detect patterns that may indicate bottlenecks, quality risks, cost movement, collaboration issues, role/activity mismatches, or weak evidence.
* **Explain likely causes** — summarize why a problem may be happening, using connected evidence rather than a single metric.
* **Generate recommendations** — draft concrete improvement actions with owner, expected metric movement, and validation window.
* **Rank improvement opportunities** — help teams focus on problems with larger expected impact, stronger evidence, or lower effort to fix.
* **Explain evidence in plain language** — make complex metric, lineage, cost, or quality findings understandable to leaders and teams.
* **Improve readiness** — identify missing sources, unresolved identities, broken lineage, weak role/activity definitions, or missing outcome signals.
* **Learn from outcomes** — compare what changed after an action was taken and use the result to improve future recommendations.

⠀
AI-generated diagnoses and recommendations are not treated as automatically true. Every recommendation declares whether it is evidence-derived from the customer’s own data or a heuristic suggestion based on configured rules or best practices. Heuristic recommendations are shown as such and are not presented as data-proven findings.

A human owner decides whether to accept, modify, reject, or act on a recommendation. Insight then tracks the follow-up window and validates whether the action improved the measured system.

### 5.1 Recommendation Examples

Insight recommendations are structured improvement objects. They connect an observed problem to evidence, action, ownership, expected movement, and validation.

**Example 1 — Engineering review bottleneck**

* **Observed problem:** Pull requests for Team A wait too long for review, and release items are delayed even though code is being completed.
* **Evidence:** Review queue time increased by 35% over the last four weeks; 42% of completed changes waited more than two business days for first review; the delay is concentrated in two services and three reviewers.
* **Diagnosis:** Delivery speed is limited by review capacity, not by code production.
* **Recommended action:** Reassign review ownership for the affected services, add backup reviewers, and set an escalation rule for review queues older than two business days.
* **Owner:** Engineering manager for Team A and service owners for the affected services.
* **Expected metric movement:** Lower median review wait time, fewer blocked work items, shorter cycle time for release-bound work.
* **Validation window:** Compare the next four weeks with the previous four weeks, while monitoring escaped defects and reopen rates to ensure quality does not degrade.

⠀
**Example 2 — Sales activity without pipeline movement**

* **Observed problem:** Sales activity increased, but qualified pipeline did not move.
* **Evidence:** Outbound emails and meetings increased by 28% over six weeks; opportunity stage movement stayed flat; follow-up time after demos increased; several accounts had repeated meetings without next-step ownership.
* **Diagnosis:** Higher activity volume is not translating into sales progress because follow-up ownership and qualification signals are weak.
* **Recommended action:** Define follow-up SLA after demos, review stalled opportunities, assign next-step owners, and update qualification criteria for the affected segment.
* **Owner:** Sales lead for the segment.
* **Expected metric movement:** Faster follow-up time, higher stage conversion, fewer stalled opportunities.
* **Validation window:** Compare the next sales cycle with the previous cycle and separate activity volume from pipeline movement.

## 6. Target Audience

Insight is used at multiple levels of the organization: executives set priorities, leaders improve teams and functions, contributors understand their own work context, and administrators configure the evidence model.

### 6.1 Target users:
The primary user groups are:

* **Executives and portfolio leaders** — see where the organization is improving, where it is only getting busier, where cost or quality is offsetting delivery gains, and where AI adoption is actually helping. They use Insight to choose improvement priorities, allocate investment, and validate whether major changes worked.
* **Functional leaders and team managers** — understand how their teams are performing, where work is blocked, which risks or cost drivers need attention, and which improvement actions can help the team work better.
* **Functional teams and individual contributors** — product, engineering, operations, support, sales, marketing, finance, and other teams understand how their work contributes to outcomes and where the system around them needs improvement. Individual contributors see their own activity and context only, without access to other people’s raw activity or stack ranking.
* **Data stewards and administrators** — configure roles, activities, integrations, metrics, access rules, localization, and evidence coverage.

### 6.2 Functional Coverage

Insight serves every function that contributes to how products and services are planned, produced, delivered, sold, supported, operated, financed, and improved.

For each function, Insight connects available evidence, finds problems, recommends improvement actions, and validates whether the actions helped. The examples below describe product capabilities, not fixed assumptions about how every customer works. Actual diagnoses and recommendations depend on connected systems, data quality, and the customer’s configured roles, activities, metrics, and governance rules.

**6.2.1 Engineering / R&D**

For Engineering and R&D, Insight connects people, teams, repositories, work items, code changes, reviews, CI/CD, releases, defects, AI usage, and development cost.

Insight helps find where engineering work slows down, where review or CI becomes a bottleneck, where AI-assisted work changes output or quality, where teams carry too much unplanned work, and where delivered work creates downstream cost or defects.

It can recommend actions such as reducing review queues, changing ownership of blocked work, improving test or release gates, splitting overloaded teams or services, changing AI usage guidance for specific work types, or investigating areas where delivery speed improved while quality degraded or support load increased.

**6.2.2 Product Management**

For Product Management, Insight connects intent, roadmap items, initiatives, committed work, delivered changes, customer impact, quality, cost, and outcome signals.

Insight helps find where roadmap intent does not translate into delivered work, where scope or effort drifts, where teams spend capacity outside planned priorities, where delivered work does not produce expected outcome movement, and where attribution confidence is too weak to support a conclusion.

It can recommend actions such as clarifying initiative ownership, changing portfolio allocation, reducing scope, improving work-to-outcome lineage, pausing low-confidence claims, or instrumenting missing outcome signals before making investment decisions.

**6.2.3 Design / UX**

For Design and UX, Insight connects design tasks, research activities, prototypes, handoffs, product requirements, implementation work, user feedback, and post-release quality or adoption signals.

Insight helps find where design work is blocked, where handoffs to product or engineering create rework, where shipped implementation diverges from intended experience, and where user feedback suggests recurring usability problems.

It can recommend actions such as improving handoff criteria, adding design review gates for specific work types, prioritizing recurring UX issues, or connecting missing feedback and research sources when evidence is incomplete.

**6.2.4 DevOps / SRE**

For DevOps and SRE teams, Insight connects CI/CD, deployments, service ownership, incidents, monitoring, on-call load, cloud cost, runtime cost, and release-to-production lineage.

Insight helps find where delivery pipelines are slow or unreliable, where production cost is increasing, where incidents are linked to recent changes, where on-call load is concentrated, and where service ownership or operational responsibility is unclear.

It can recommend actions such as improving pipeline reliability, changing deployment or rollback practices, assigning missing service ownership, reducing specific cost drivers, improving observability, or feeding incident patterns back into product and engineering work.

**6.2.5 QA / Quality Engineering**

For QA and Quality Engineering, Insight connects test execution, test coverage signals, build health, defects, reopened work, escaped defects, release readiness, and production feedback.

Insight helps find where quality is degrading, where flaky tests or unstable builds slow delivery, where AI-assisted code needs stronger review or test coverage, and where post-release issues reveal gaps in pre-release validation.

It can recommend actions such as strengthening release gates, prioritizing defect categories, improving test coverage for specific work types, reducing flaky tests, or changing review requirements when speed increases but quality does not hold.

**6.2.6 Support / Customer Operations**

For Support and Customer Operations, Insight connects tickets, escalations, SLA performance, reopen rates, handling time, customer feedback, knowledge gaps, product areas, releases, and delivered work.

Insight helps find where support load is increasing, where issues repeat, where tickets are linked to recent releases or product changes, where knowledge gaps create avoidable handling time, and where customer pain is not reaching product or engineering teams.

It can recommend actions such as updating documentation, improving routing or staffing, escalating recurring root causes to product owners, linking support load back to delivered features, or changing release readiness rules when post-release tickets, escalations, or reopen rates increase.

**6.2.7 Sales**

For Sales, Insight connects CRM data, opportunities, pipeline movement, customer communication, meetings, demos, follow-ups, proposals, presentations, email, calendar, Zoom/Teams calls, sales-engagement tools, and deal outcome signals.

Insight helps find where sales effort does not convert, where deals stall, where follow-up is slow, where activity volume hides weak pipeline movement, where handoff from marketing or product is poor, and where AI-generated outreach increases activity without improving outcomes.

It can recommend actions such as changing follow-up SLAs, improving opportunity qualification, reviewing stalled deal stages, changing account ownership, refining sales playbooks, or instrumenting missing communication sources before drawing conclusions about sales performance.

**6.2.8 Marketing**

For Marketing, Insight connects campaigns, content production, audience engagement, lead generation, lead quality, attribution signals, handoff to sales, pipeline movement, and campaign cost.

Insight helps find where campaign volume does not create qualified pipeline, where content production does not lead to downstream movement, where leads are rejected or stalled after handoff, where attribution is weak, and where AI-assisted content increases output without improving business outcomes.

It can recommend actions such as improving qualification rules, changing campaign mix, prioritizing content types with stronger downstream effect, fixing sales handoff gaps, reducing low-impact activity, or improving attribution before claiming campaign impact.

**6.2.9 Finance / FinOps**

For Finance and FinOps, Insight connects people cost, tool and license cost, AI cost, cloud cost, production cost, support cost, billing data, and allocation rules.

Insight helps find where cost is unattributed, where allocation rules are weak, where AI or cloud spend is concentrated, and where finance lacks enough lineage to explain which teams, products, services, or work items created the cost.

It can recommend actions such as improving allocation rules, separating seat-based and usage-based AI cost, investigating cost anomalies, reducing unused spend, or improving cost lineage before making budget, chargeback, or ROI decisions.

### 6.3 Cross-functional Improvement Areas

Some problems do not belong to one function. They appear between teams, roles, systems, and handoffs. Insight treats these as cross-functional improvement areas rather than as separate departments.

**6.3.1 Communication Load and Collaboration**

Insight connects signals from email, calendar, meetings, chat, documents, comments, reviews, approvals, handoffs, and shared work items.

It helps find where communication load is too high, where teams spend too much time coordinating instead of progressing work, where meetings or message volume do not lead to decisions, and where lack of collaboration blocks delivery, sales, support, operations, or customer outcomes.

It can recommend actions such as reducing recurring meetings, clarifying decision ownership, improving handoff rules, changing escalation paths, creating shared ownership for cross-functional work, or connecting missing communication sources before drawing conclusions about collaboration quality.

**6.3.2 Handoffs and Ownership**

Insight connects work items, approvals, reviews, escalations, service ownership, customer requests, support tickets, incidents, and release or delivery events.

It helps find where work waits between teams, where ownership is unclear, where decisions are delayed, where the same issue moves across functions without resolution, and where local progress does not translate into end-to-end movement.

It can recommend actions such as assigning clear ownership, changing approval rules, reducing handoff steps, creating escalation paths, or redefining responsibility between teams when work repeatedly stalls.

**6.3.3 AI Adoption and Impact**

Insight connects AI usage with work performed, output, quality, cost, flow, and downstream outcomes.

It helps find where AI adoption improves real work, where it only increases activity volume, where AI-assisted work adds review effort, quality risk, support load, or cost, and where teams need different guidance for different work types.

It can recommend actions such as changing AI usage guidance, reviewing AI-assisted work patterns, improving prompts or workflows, adjusting review and test gates, reducing low-value AI usage, or validating AI impact before scaling adoption.

**6.3.4 Cost-to-Outcome Flow**

Insight connects people cost, tool cost, AI cost, cloud cost, production cost, support cost, work performed, delivered changes, and outcome signals.

It helps find where cost increases without matching output or outcome movement, where savings in one area shift cost, support load, quality risk, or operational effort elsewhere, where production or support cost is linked to delivered work, and where cost attribution is too weak for confident decisions.

It can recommend actions such as improving cost allocation, investigating cost anomalies, changing investment allocation, reducing unused spend, or instrumenting missing cost lineage before making ROI claims.

**6.3.5 Evidence Coverage and Instrumentation**

Insight connects source coverage, connector health, identity resolution, role/activity configuration, metric definitions, and evidence gaps.

It helps find where conclusions are weak because key systems are missing, identities are unresolved, roles are outdated, activities are poorly configured, or outcome signals are not connected.

It can recommend actions such as connecting missing systems, improving identity resolution, refining role and activity definitions, adding outcome signals, or marking conclusions as low-confidence until the evidence is strong enough.

### 6.4 Target Companies

#### **6.4.1 Product Type**

Constructor Insight is designed for organizations that produce, deliver, operate, sell, support, or finance complex products and services.

The product is not limited to a specific business model or industry. Insight is most useful where work is performed across multiple roles, systems, teams, and cost centers, and where leaders need to understand how work, quality, cost, AI usage, and outcomes affect each other.

#### **6.4.2 Company Size**

Insight is intended to scale across several organization sizes:

* **Small teams:** 5–50 people involved in product, service, operational, commercial, or support work.
* **Mid-size organizations:** 50–500 people across product, engineering, support, sales, marketing, operations, and finance.
* **Large organizations:** 500–5,000 people with multiple teams, products, systems, and management layers.
* **Enterprise organizations:** 5,000+ people, multiple business units, multiple operating models, and stronger requirements for governance, localization, deployment choice, and cost allocation.

Scale is defined not only by employee count, but also by number of connected systems, repositories, work items, events, products, services, roles, teams, and years of retained history.

### 6.5 Maturity Level

Insight supports organizations at different measurement and improvement maturity levels.

* **Greenfield** — new products or teams can define roles, activities, source systems, outcome signals, cost allocation, and improvement workflows from the start. Insight helps make the improvement loop — measurement, diagnosis, recommendation, and validation — part of the operating model before launch.
* **Brownfield** — existing organizations can connect the systems they already use, work with imperfect evidence, surface coverage gaps, and strengthen conclusions progressively. In readiness mode, Insight identifies missing sources, unresolved identities, weak role/activity definitions, and disconnected outcome signals before making strong recommendations.

### 6.6 Data, System, and Organization Scale

Insight is useful from a small team with a few connected systems to a large organization spanning many products, functions, source systems, repositories, services, cost centers, and business units.

Scale is not defined only by codebase size. It also includes the number of people, roles, teams, connected tools, work items, communication signals, customer interactions, incidents, support tickets, cost records, AI usage events, and years of retained history.

Across that span, Insight needs identity resolution, role history, source coverage, metric consistency, evidence confidence, and cost allocation to remain coherent. Detailed deployment scale targets belong in the technical architecture and deployment documentation.
### 6.7 Target Organization Types

Insight is designed for organizations that need to improve work across multiple roles, systems, teams, and cost centers. Typical environments include:

1. **AI-adopting organizations** — companies rolling out AI tools across teams and needing to understand whether AI improves real work, quality, cost, and outcomes.
2. **Product and service organizations** — companies that produce, deliver, sell, support, operate, or finance complex products or services across multiple functions.
3. **Organizations with fragmented measurement systems** — companies replacing disconnected dashboards, local analytics, manual reports, or internally built measurement tools with one governed intelligence product.
4. **Multi-unit organizations** — companies operating across multiple products, departments, regions, business units, or cost centers that need consistent measurement, diagnosis, recommendation, and validation.
5. **Organizations modernizing legacy operating models** — companies improving old processes, unclear ownership, weak instrumentation, or disconnected source systems while continuing to run existing work.


## 7\. Work Lifecycle and Shared Information Model

Insight provides intelligence across the full lifecycle of work, not only from request to delivery. Work starts with intent — what the organization is trying to achieve — and continues through production, customer use, support, cost, learning, and improvement.

### 7.1 Lifecycle View

Insight reasons over three broad lifecycle phases:

1. **Plan** — decide what should be done, why it matters, who owns it, and what outcome is expected.
2. **Execute** — perform the work across roles, systems, teams, tools, and handoffs.
3. **Operate and Improve** — run the result, support users or customers, observe cost and quality, learn from outcomes, and improve the system.

⠀
Insight observes all three phases where source evidence exists. Where evidence is missing, Insight shows the gap instead of presenting a complete claim.

### 7.2 Shared Information Model

Insight uses one shared information model so that metrics, diagnoses, recommendations, and dashboards reason over the same entities instead of redefining them in each view.

Core entities include:

* **Person** — may hold multiple roles at once and may change roles over time.
* **Role** — a customer-configured function defined by expected activities.
* **Activity** — work a role is expected to perform; observed activity can be compared with the expected role model.
* **Team / Org unit** — an operational or reporting structure with historical membership.
* **Work item / initiative / request** — a unit of planned or committed work.
* **Change / review / validation / release / deployment** — delivery events where relevant.
* **Customer interaction / opportunity / campaign / ticket / incident** — downstream work and outcome signals outside engineering.
* **Cost record / AI usage event** — observable cost and AI activity linked to people, teams, systems, work, or outcomes where evidence allows.

⠀
Role and activity are separate axes. A customer can change role definitions, split roles into more granular roles, update expected activities, and preserve historical analysis under the role model that was valid at the time.

### 7.3 Lineage Before Attribution

Insight follows work through the systems it passes. Lineage is the ability to connect a unit of work, activity, cost, or outcome across source systems.

Examples of lineage include:

* engineering delivery: intent → work item → code change → review → validation → release → deployment → incident;
* support: ticket → escalation → resolution → product area or release;
* sales and marketing: campaign → lead → opportunity → deal movement → close or loss;
* finance and cost: cost record → system, team, product, service, work item, or cost center.

⠀
Lineage comes before attribution. Insight does not strongly attribute cost, quality, AI impact, or outcome movement to work it cannot trace. Broken or weak links are shown as evidence gaps, not silently converted into confident claims.

### 7.4 Release and Deployment

Insight treats release and deployment as separate entities.

A **release** is a versioned, deployable package or product version. A **deployment** is the installation, configuration, or rollout of that release into a specific environment or customer context.

This distinction matters for cloud, private cloud, on-prem, IaaS, and customer-operated environments, where the same release may be deployed at different times, in different configurations, with different operational cost, support load, and customer impact.

### 7.5 Readiness and Data Quality

Insight assumes that evidence quality improves over time. It can start with incomplete data, show what is already credible, and identify what must be connected or resolved before stronger conclusions are made.

Readiness mode helps customers find the smallest set of fixes with the largest improvement in confidence: missing sources, unresolved identities, weak role/activity definitions, disconnected outcome signals, or broken lineage.

As evidence improves, Insight can move from directional signals to stronger diagnoses and recommendations while keeping confidence and limitations visible.

## 8\. Product Capabilities

Insight provides the following product capabilities:

1. **Source connection and evidence coverage** — connect the systems a customer already uses, show what evidence is available, and identify gaps that limit confidence.
2. **Identity, role, and organization model** — resolve people across systems, support temporal team membership, configurable roles, multiple roles per person, and role changes over time.
3. **Work, outcome, and cost lineage** — connect work, delivery, operations, support, customer outcomes, AI usage, and cost where evidence allows.
4. **Measurement and metric definitions** — maintain governed definitions for metrics, units, granularity, thresholds, confidence, and limitations.
5. **Analysis and diagnosis** — detect bottlenecks, risks, anomalies, cost drivers, quality issues, weak evidence, and role/activity mismatches.
6. **Recommendation and validation** — suggest improvement actions, show evidence and confidence, identify an owner, and validate whether the action helped.
7. **Customer configuration** — let customers configure roles, activities, source systems, metrics, thresholds, dashboards, cohorts, access rules, localization, and governance policies.
8. **Exposure and consumption** — provide Insight views, summaries, dashboards, APIs, and governed data access for customers who want to use Insight outputs in other systems.

Recommendations declare whether they are evidence-derived from the customer’s own data or heuristic suggestions based on configured rules or best practices. Heuristic recommendations are shown as such and are not presented as data-proven findings.

These capabilities can mature progressively. A customer can start with partial evidence, use readiness mode to improve coverage, and move from directional signals to stronger diagnoses and recommendations over time.

## 9\. Customer Configuration

Customer configuration is a core product capability, not a services engagement. Insight must adapt to how each customer actually works: their roles, activities, systems, metrics, governance rules, language, and operating model.

Customers can configure:

* **Roles** — define, create, split, merge, rename, and retire roles used in their organization.
* **Activities per role** — define expected activities for each role and compare them with observed work patterns.
* **Role assignment history** — assign one or multiple roles to a person and preserve historical role membership over time.
* **Org structure and teams** — map teams, departments, business units, reporting lines, and temporal membership.
* **Source systems** — choose which systems are connected and what evidence each source is allowed to provide.
* **Metrics and thresholds** — choose which metrics matter, define thresholds, and adapt them by function, team, tenant, or time period.
* **Cohorts and comparison groups** — define who is compared with whom and which baselines are valid.
* **Recommendations** — configure which recommendation families are enabled, who owns them, and how validation windows are defined.
* **Dashboards and views** — compose role-specific and function-specific views from the metric and recommendation catalog.
* **Access rules** — define who can see raw data, people-level data, aggregate data, cost data, recommendations, and evidence details.
* **Localization** — configure language, date format, time format, number format, currency, timezone, and regional display rules.

⠀
Insight also helps customers refine configuration over time. When observed work does not match the configured role model, when important activities are missing, or when evidence is too weak for confident diagnosis, Insight can recommend configuration changes instead of hiding the mismatch.
## 10\. Integrations

Insight reads from the systems a company already uses. Evidence categories matter more than vendor names: Insight integrates with systems that provide the required evidence for people, work, collaboration, delivery, operations, support, sales, marketing, cost, AI usage, and outcomes.

The product is open and extensible. New connectors can be added as new evidence needs appear. A system can be integrated through an API, export, webhook, database access, event stream, or open standard such as OpenTelemetry.

The categories and products below are examples, not a closed list. Equivalent systems in the same evidence category can be supported. If an important evidence category is missing or weak, Insight reports it as a coverage gap instead of assuming the data exists.

### 10.1 People, Organization, and Identity

* **HR systems** — people, employment status, role, department, manager, location, cost basis, start/end dates, and role history.
* **SSO / identity providers** — account mapping, user identities, group membership, and cross-system identity resolution.
* **Org charts / directory systems** — reporting lines, teams, business units, and temporal membership.

⠀
Examples: Workday, BambooHR, Entra ID, Okta, Google Workspace, Microsoft 365, and equivalents.

### 10.2 Work, Planning, and Product Management

* **Work trackers and planning systems** — work items, states, priorities, types, estimates, complexity, ownership, dependencies, and links to changes.
* **Roadmap and portfolio systems** — initiatives, commitments, planning horizons, strategic themes, and portfolio allocation.
* **Knowledge and documentation systems** — requirements, decisions, ownership context, documentation, and knowledge gaps.

⠀
Examples: Jira, YouTrack, GitHub Issues, GitLab Issues, Linear, Azure DevOps Boards, Asana, Trello, Confluence, Notion, wikis, and equivalents.

### 10.3 Collaboration and Communication

* **Email, calendar, chat, meetings, and documents** — communication load, meetings, handoffs, decisions, follow-ups, shared documents, comments, and collaboration signals.

⠀
Examples: Outlook, Gmail, Google Calendar, Microsoft Teams, Slack, Zoom, Google Meet, shared drives, document systems, and equivalents.

Insight uses these sources to understand collaboration patterns and handoff health, not to expose private message content by default. Access and visibility follow the customer’s governance rules.

### 10.4 Source Control, Review, and CI/CD

* **Source control and code review systems** — changes, authorship, reviews, comments, approvals, merge events, and links to work.
* **CI/CD and release systems** — builds, tests, pipeline duration, failures, releases, deployments, and environment signals.

⠀
Examples: GitHub, GitLab, Bitbucket, Azure DevOps, Jenkins, GitHub Actions, GitLab CI, and equivalents.

### 10.5 AI Tool Telemetry

* **AI assistants, coding tools, and AI APIs** — sessions, prompts or task metadata where available, model/tool identity, token usage, accepted suggestions, generated code or content signals, and AI cost.

⠀
Examples: OpenAI, Anthropic Claude / Claude Code, Microsoft Copilot, Cursor, JetBrains AI, internal model gateways, and equivalents.

### 10.6 Operations, Reliability, and Observability

* **Monitoring, observability, incident, and on-call systems** — incidents, alerts, severity, ownership, resolution, service health, runtime behavior, deployment impact, on-call load, and operational effort.
⠀
Examples: Prometheus, Grafana, Datadog, New Relic, PagerDuty, Opsgenie, Sentry, OpenTelemetry-compatible sources, and equivalents.

### 10.7 Support and Customer Operations

* **Support and customer operations systems** — tickets, escalations, SLA performance, reopen rates, handling time, customer/service mapping, knowledge gaps, post-release support load, and customer impact.

⠀
Examples: Zendesk, Intercom, ServiceNow, Freshdesk, Jira Service Management, and equivalents.

### 10.8 Sales and Marketing

* **CRM, sales engagement, meetings, proposals, campaigns, and marketing systems** — pipeline movement, opportunities, account activity, customer communication, demos, follow-ups, proposals, presentations, campaigns, leads, attribution, handoff to sales, and outcome signals.

⠀
Examples: Salesforce, HubSpot, Outreach, Salesloft, Gong, Chorus, Marketo, Google Analytics, campaign platforms, presentation/document systems, and equivalents.

### 10.9 Cost, Billing, and Finance

* **Cloud, infrastructure, AI, software, billing, and finance systems** — compute, storage, network, AI usage cost, seats, licenses, contracts, currency, billing period, cost centers, allocation rules, and unattributed cost.

⠀
Examples: AWS, Azure, GCP, cloud billing exports, finance systems, procurement systems, SaaS management systems, and equivalents.

### 10.10 Connector Evidence Contract

Every connector declares what it can and cannot prove before its fields support a product claim. At minimum, a connector declares:

* source system and evidence category;
* available fields and units;
* time range and freshness;
* identity mapping assumptions;
* coverage and known blind spots;
* whether the evidence supports measurement, diagnosis, recommendation, or validation.

⠀
If the evidence is incomplete, Insight can still use it as a directional signal, but conclusions remain confidence-rated and limitations stay visible.


## 11\. Cost Model
Insight connects cost to work, systems, roles, teams, products, services, and outcomes. Cost is not shown only as a finance total; it is used to understand where work becomes expensive, where cost moves between parts of the system, and whether improvement actions actually reduce or shift cost.
### 11.1 Cost Categories
Insight assembles cost across the major areas where work is created, delivered, operated, supported, and improved:

**Total observable cost = People cost + Tooling cost + AI cost + Infrastructure cost + Production cost + Support cost + Operations cost**

The exact categories and allocation rules are customer-configurable. At minimum, Insight distinguishes:
* **People cost** — effort from employees, contractors, and teams contributing to work across functions.
* **Tooling and license cost** — development tools, collaboration tools, AI tools, SaaS systems, seats, subscriptions, and contracts.
* **AI cost** — AI seats, API/token usage, model gateway cost, and self-hosted model compute.
* **Infrastructure and compute cost** — CI runners, test environments, development environments, build infrastructure, storage, and network.
* **Production cost** — runtime compute, storage, network, observability, operations effort, and production AI inference where applicable.
* **Support cost** — support people, ticket handling, escalations, configuration, upgrades, customer operations, and post-release effort.

⠀Insight preserves unattributed cost instead of forcing it into precise-looking totals. Each cost view declares its allocation rule, evidence, and confidence.
### 11.2 AI and Compute Cost
AI cost can be analyzed as its own view, but it is also allocated to the part of the operating system where it is incurred: development, product work, sales, marketing, support, operations, or production.
AI cost appears in several forms:
* **Per-seat subscriptions** — fixed licenses for AI tools, charged per user or seat.
* **Hosted model usage** — token, API, or usage-based cost from hosted AI providers.
* **Self-hosted model compute** — GPU/CPU cost for inference, fine-tuning, training, or internal model serving.
* **Embedded production AI** — AI consumed by product features or services after release.

⠀Ordinary compute that does not run an AI model — CI runners, test environments, build systems, application servers, storage, and network — remains regular infrastructure, development, or production cost. The distinction is based on what the compute is used for, not on whether it is expensive or cheap.
When AI shifts cost between areas — for example, development effort falls but production inference, review, maintenance, or support cost rises — Insight preserves that movement instead of collapsing it into one generic “AI value” number.
### 11.3 AI Exposure, Cost, and Impact
Insight analyzes AI usage through three layers:
| **Layer** | **Question** | **Evidence needed** | **Supports the statement** |
|:-:|:-:|:-:|:-:|
| **Exposure** | Where and how much was AI used? | Sessions, tokens, tool identity, actor, team, time, accepted suggestions or generated output where available | “AI was used in this scope, to this extent.” |
| **Cost** | What did that usage cost? | Seat prices, token/API usage, compute usage, pricing rules, allocation rules | “AI cost for this scope was X, with stated limits.” |
| **Impact** | What changed when AI was used? | Lineage, baseline, comparison group, quality, cost, flow, and outcome context | “AI-exposed work moved differently in this bounded comparison.” |
These layers apply to AI used by people while performing work and to AI consumed by products or services in production. Insight does not assume impact from usage alone. Where impact evidence is missing, it shows what is missing and can recommend what to instrument next.
### 11.4 Cost-to-Outcome Analysis
Insight does not treat lower cost as the only goal. It analyzes cost together with output, quality, flow, support load, operational effort, and outcomes.

It helps find cases where:
* cost increases without matching output or outcome movement;
* local savings shift cost, quality risk, support load, or operational effort downstream;
* AI adoption increases activity but does not improve outcomes;
* production or support cost is linked to specific releases, services, teams, or work types;
* cost is too weakly attributed to support a confident ROI claim.

⠀Insight can recommend actions such as improving allocation rules, reducing unused spend, changing AI usage guidance, optimizing infrastructure, investigating cost anomalies, or improving cost lineage before making ROI or investment claims.



## 12\. Adoption and Extension

### 12.1 Adoption Path

Insight can start with the systems and evidence a customer already has. The customer does not need perfect coverage before getting value; the product starts with available evidence, shows gaps, and strengthens conclusions as coverage improves.

A typical adoption path is:

1. **Connect existing systems** — start with available sources across people, work, collaboration, delivery, support, operations, sales, marketing, finance, AI usage, and cost.
2. **Configure the operating model** — define roles, activities, teams, org structure, metrics, thresholds, access rules, localization, and recommendation ownership.
3. **Run readiness analysis** — identify missing sources, unresolved identities, broken lineage, weak role/activity definitions, and disconnected outcome signals.
4. **Start with directional insight** — use available evidence for initial measurement, diagnosis, and recommendations, with confidence and limitations shown.
5. **Improve evidence over time** — strengthen connectors, identity resolution, lineage, cost allocation, and outcome signals.
6. **Validate improvement actions** — track whether recommended or customer-selected actions improved output, quality, cost, flow, collaboration, or outcomes.

⠀
### 12.2 Extension Points

Insight is designed to be extended as customers add systems, roles, metrics, and improvement needs.

Extension points include:

1. **Connectors** — add new source systems and evidence categories.
2. **Metric definitions** — add or refine metrics, units, thresholds, granularity, and limitations.
3. **Role and activity models** — create, split, merge, or refine roles and expected activities.
4. **Identity and lineage rules** — improve cross-system identity resolution, team membership, work lineage, and cost attribution.
5. **Diagnosis rules** — add new bottleneck, anomaly, quality, collaboration, cost, or risk patterns.
6. **Recommendation logic** — add recommendation families for new roles, functions, processes, and improvement areas.
7. **Validation rules** — define how actions are evaluated after implementation.
8. **Migration paths** — move fragmented internal metrics, dashboards, scripts, and reporting systems into a governed Insight model.

⠀
Extensions can be customer-specific or contributed back into shared product capabilities when appropriate. No extension should create a confident metric, diagnosis, or recommendation without declared evidence, coverage, and limitations.

## 13\. Non-Functional Requirements and Migration
### 13.1 Non-Functional Requirements
Insight must be usable as an operational product, not a one-off analytics project. Non-functional requirements are product design targets and are validated per deployment model.
| **Dimension**                    | **Requirement**                                              |
|:--------------------------------:|:------------------------------------------------------------:|
| **Scale**                        | Reference deployment target: ~10K users and ~10M ingested events/day. Larger deployments scale through additional storage, compute, partitioning, or multiple governed deployments where needed. |
| **Data ingestion**               | Support scheduled ingestion from connected systems, with separate historical backfill for large existing data sets. Insight is not designed as a real-time operational telemetry pipeline. |
| **Data freshness**               | Show when each source, metric, diagnosis, recommendation, and view was last updated. Stale evidence must be visible to users. |
| **Performance**                  | Keep user-facing views interactive by using precomputed metrics, diagnoses, recommendations, and lineage summaries where appropriate. |
| **Availability**                 | Support routine maintenance, connector failures, and partial source outages without taking the whole product down. Availability targets are defined per deployment model. |
| **Correctness and auditability** | Keep metrics, diagnoses, recommendations, and cost allocations reproducible from source evidence. Avoid silent double-counting and preserve calculation methods, versions, and limitations. |
| **Security and isolation**       | Enforce tenant isolation, encryption in transit and at rest, role-based access, and separation between raw, aggregate, people-level, cost, and recommendation data. |
| **Deployment**                   | Support Constructor-hosted cloud, customer cloud, private cloud, and customer-operated installation, including environments with strict data residency or network constraints. |
| **Upgradeability**               | Support customer-safe upgrades, configuration migration, versioned rules, and backward-compatible access to historical data where possible. |
| **Localization**                 | Support customer and user-level localization: language, date format, time format, number format, currency, timezone, regional display rules, and right-to-left text where required. |
Detailed scale ceilings, validated throughput numbers, and hardware profiles should be maintained in the technical architecture and deployment documentation, not asserted as universal limits in the Vision document.

### 13.2 Migration and Historical Data
Insight can replace fragmented internal measurement, reporting, analytics, and cost-tracking systems over time. Migration is treated as a controlled product adoption path, not a one-time data import.

The first migration targets are Constructor, Acronis, and Virtuozzo. Each starts from a different measurement landscape, so the migration plan is different for each company.
| **Company** | **Current internal measurement** | **Intent** |
|:-:|:-:|:-:|
| **Constructor** | Multiple internal analytics systems, including development metrics and AI-tool usage/cost | Consolidate onto Insight |
| **Acronis** | A developer-productivity analytics system, now in maintenance | Migrate its historical data, then replace once parity is reached |
| **Virtuozzo** | Current analytics on a general-purpose BI tool; no dedicated legacy product | Deploy Insight as the analytics layer, alongside existing BI until ready |
A typical migration includes:
1. **Inventory existing systems** — identify current dashboards, internal analytics, scripts, reports, source systems, metrics, and owners.
2. **Map required metrics and views** — decide which existing metrics must be preserved, retired, renamed, or replaced by better definitions.
3. **Import historical data where available** — retain historical events and metric history when source retention and data quality allow it.
4. **Validate parity where required** — compare legacy and Insight outputs for an agreed historical period before retiring a legacy system.
5. **Expose gaps honestly** — show where old metrics cannot be reproduced because source data is missing, definitions were inconsistent, or identity/lineage was weak.
6. **Move users gradually** — transition teams, leaders, and administrators to Insight views, recommendations, and workflows.
7. **Retire legacy systems safely** — shut down old systems only after required metrics, connectors, stewardship, access rules, and migration owners are in place.

⠀Historical comparison depends on source retention. Some systems preserve years of history; others retain only short windows. Insight shows these limits explicitly so users do not compare periods with unequal evidence.



## Glossary

Plain-language definitions; terminology is consistent across this document.

| Term                     | Definition                                                   |
|--------------------------|--------------------------------------------------------------|
| **Intent**               | What the organization intends to build (replaces "request"); the start of the lifecycle. |
| **Lineage**              | The traceable path of a unit of work through the systems it passes. Attribution cannot be stronger than lineage. |
| **Attribution**          | How a value (cost, incident, effect) is tied to work — *direct* (explicit link), *derived* (declared inference rule), *allocated* (declared split rule), or *unknown* (reported separately). Insight computes it; users may override; it can differ by period, scope, or role. |
| **Confidence**           | How strong the evidence is for a claim — strong / medium / weak. Computed from coverage and link quality; thresholds configurable per metric family. |
| **Cohort**               | A defined group of people or teams compared as one unit — a squad, a role, AI-exposed vs. not — with the comparison design declared. |
| **Evidence**             | The underlying source records (events, work items, changes, cost lines) a metric, diagnosis, or recommendation is computed from and can be opened and inspected. |
| **Coverage**             | How completely the connected sources supply the data a view needs — a data/source-completeness measure, never a statement about which roles Insight serves. |
| **Diagnosis**            | Interpretation of *why* a result occurred and *how serious* it is — bottlenecks, risks, anomalies — going beyond reporting the number. |
| **Recommendation**       | An evidence-backed, confidence-rated suggestion on process, tooling, staffing, or cost. A suggestion only; a human decides and acts. |
| **Provenance**           | For a recommendation, whether it is *evidence-derived* (from this organization's own data and lineage) or a *heuristic / best-practice default*. |
| **Granularity (grain)**  | The lowest level at which a metric may be read — person, team, work item, feature, release, service, period. |
| **Tenant**               | One customer organization in a shared (multi-tenant) deployment; data and configuration are isolated per tenant. |
| **Exposure**             | Observable AI activity (sessions, tokens, accepted lines). Activity evidence — not spend or value. |
| **Readiness mode**       | The state used when evidence is too weak for a strong claim — shows the gaps and what to fix, not a fabricated number. |
| **Release / Deployment** | A *Release* is a versioned, deployable package; a *Deployment* is installing and configuring it into an environment. |
| **Observable cost**      | Development + production + support cost, with AI cost as a subcomponent. |
