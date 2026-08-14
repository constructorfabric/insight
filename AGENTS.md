<!-- @cf:root-agents -->
```toml
cf-studio-path = ".cf-studio"
```

ALWAYS resolve and enforce prerequisites of skills/workflows/commands BEFORE applying user intent.
<!-- /@cf:root-agents -->

## Project Rules

NEVER put an identifier containing `_` in a Markdown heading a table of contents links to — the TOC generator drops the underscore while GitHub keeps it, so the anchor breaks (markdownlint MD051). Word the heading in prose and keep the identifier in the body.

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

## Comments

- No comments unless they express a constraint the code cannot.
- Prefer code over prose: name things clearly, extract functions, use types, make invalid states unrepresentable.
- Allowed comments are brief and tagged:
  - `SAFETY:` — non-obvious safety/security/correctness reasoning.
  - `INVARIANT:` — a fact a future edit could silently break.
  - `WORKAROUND:` — external/platform/dependency behavior being worked around.
  - Tool/linter/compiler suppressions — brief adjacent justification.
- Do not comment:
  - implementation history or how the code got here;
  - what the code already says;
  - alternatives considered or roads not taken;
  - issue/PR context, phase notes, headers, or discussion history.
- Non-obvious semantics belong in types, tests, or docs when possible.
- Doc comments only for genuinely public/external APIs; keep them brief.
- Comments should normally be one or two lines. If a comment needs a paragraph, improve the code or move the rationale to documentation.
- If deleting a comment does not materially reduce safety, correctness, or maintainability, delete it when it is within the scope of the current work.