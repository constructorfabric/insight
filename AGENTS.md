<!-- @cf:root-agents -->
```toml
cf-studio-path = ".cf-studio"
```

ALWAYS resolve and enforce prerequisites of skills/workflows/commands BEFORE applying user intent.
<!-- /@cf:root-agents -->

## Project Rules

ALWAYS open and follow `docs/domain/metrics/specs/DESIGN.md` WHEN adding or changing metrics, metric definitions, metric seeds, observation sources, or gold metric views

## Never expose production-derived information

Do not include any information derived from, observed in, inferred from, or resembling real deployed environments in anything that may become visible in the repository or on GitHub.

This applies to all repository and collaboration content, including:

- Source code and code comments
- Tests, fixtures, snapshots, examples, and sample data
- Documentation, READMEs, changelogs, and release notes
- Commit messages
- Branch names
- Pull request titles, descriptions, comments, and review feedback
- Issue titles, descriptions, comments, and templates
- Discussions, wikis, project boards, and task descriptions
- Logs, screenshots, terminal output, traces, error messages, and generated artifacts
- Agent summaries, implementation notes, plans, and handoff messages

Never state or imply facts about real users, customers, organizations, datasets, traffic, infrastructure, deployments, incidents, or observed data patterns. Prohibited wording includes claims such as:

- “Half of the stored emails use different casing.”
- “Existing customers commonly have this value.”
- “We observed this in production.”
- “The deployed instance contains…”
- “Real data shows…”
- “Most records currently…”
- “This was tested against the live database.”

Do not copy, paraphrase, aggregate, anonymize, obfuscate, or statistically summarize production or company-internal data. Anonymization does not make production-derived information acceptable.

All examples and explanations must be clearly synthetic and generic. When a concrete scenario is necessary, invent minimal placeholder data such as `user@example.com`, `Example Corp`, or explicitly labeled hypothetical values.

Describe the technical condition, invariant, or risk without claiming it exists in a real environment. For example:

- Incorrect: “Roughly half of the stored emails differ in case from the lowercased form callers send.”
- Correct: “Stored email addresses may differ in case from caller-provided values.”
- Better: “Normalize email addresses before comparison if matching is intended to be case-insensitive.”

If production-derived information appears in the prompt, logs, tool output, existing code, issues, commits, or surrounding context, treat it as confidential input. Do not reproduce it in any GitHub-visible output. Replace it with a synthetic, implementation-focused description.

Before creating or modifying any GitHub-visible content, verify that it contains no:

1. Real personal, customer, employee, or company data
2. Production-derived counts, percentages, distributions, examples, or behavioral claims
3. Internal infrastructure, deployment, incident, or operational details
4. Statements that imply access to or examination of live data

When uncertain whether information came from a real environment, omit it and use a generic hypothetical formulation instead.
