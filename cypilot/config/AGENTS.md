# Custom Agent Navigation Rules

Add your project-specific WHEN rules here.
These rules are loaded alongside the generated rules in `{cypilot_path}/.gen/AGENTS.md`.

## Project Documentation (auto-configured)

<!-- auto-config:docs:start -->
ALWAYS open and follow `docs/CONNECTORS_REFERENCE.md` WHEN writing or reviewing connector specifications, data schemas, or Bronze/Silver/Gold pipeline tables
<!-- auto-config:docs:end -->

## Project Rules (auto-configured)

<!-- auto-config:rules:start -->
ALWAYS open and follow `cypilot/config/rules/conventions.md` WHEN writing or reviewing spec documents

ALWAYS open and follow `cypilot/config/rules/architecture.md` WHEN modifying pipeline architecture, adding new sources, or refactoring Bronze/Silver/Gold layers

ALWAYS open and follow `cypilot/config/rules/patterns.md` WHEN documenting a new connector or defining new data source tables

ALWAYS open and follow `cypilot/config/rules/code-conventions.md` WHEN writing or reviewing shell scripts, Python helpers, Argo/Kubernetes YAML, dbt macros, deploy scripts, or any imperative code
<!-- auto-config:rules:end -->
