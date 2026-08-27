-- Heal double-encoded `items` in bronze_jira.jira_issue_history.
--
-- Why: the jira_issue_history stream renders `items` with `| tojson`, and the
-- CDK re-parses that render with ast.literal_eval. JSON `null` is not a Python
-- literal, so any changelog entry whose previous value was empty stayed a
-- string; under the stream's former `anyOf: [string, array]` declaration the
-- ClickHouse destination JSON-encoded that string, storing an escaped blob:
--
--   "[{\"field\": \"status\", \"from\": null, \"toString\": \"Open\"}]"
--
-- JSONExtractArrayRaw reads such a value as a scalar, not an array, so
-- staging.jira_changelog_items silently skipped every affected row and the
-- whole field-history journal lost each field's first value. The stream now
-- declares `items` as a plain string, which stores the render verbatim; this
-- migration repairs the rows written before that fix.
--
-- Idempotent: after the first pass no value starts with a quote, so re-runs
-- (apply-ch-migrations.sh keeps no bookkeeping and replays every file) match
-- nothing. `items` is not part of the sorting key, so the column is mutable.
--
-- mutations_sync=1 keeps the deploy deterministic: the hook returns only once
-- the rewrite is applied, so the first transform after deploy reads healed
-- rows rather than racing a background mutation.

ALTER TABLE bronze_jira.jira_issue_history
    UPDATE items = JSONExtractString(items)
    WHERE startsWith(items, '"')
    SETTINGS mutations_sync = 1;
