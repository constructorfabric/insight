# wiki dbt tests

Singular SQL tests on `bronze_confluence.*` and `silver.class_wiki_*`.
Each file returns rows **that represent a violation** — a test passes
when zero rows are returned.

Run:
```bash
dbt test --select assert_comment_replies_parent_resolves --profiles-dir .
```

## What's covered

| Test | What it catches |
|------|-----------------|
| `assert_comment_replies_parent_resolves` | A reply row in `wiki_*_comment_replies` whose `parent_comment_id` doesn't resolve to an existing `comment_id` in the corresponding top-level stream — i.e. orphan reply, miswired SubstreamPartitionRouter, or cross-kind attribution. AC#4 from issue #285. |
