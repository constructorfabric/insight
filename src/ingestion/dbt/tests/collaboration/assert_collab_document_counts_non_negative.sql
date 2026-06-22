{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Collab document activity counts are non-negative',
        'domain': 'collab',
        'category': 'physical_bound',
        'tier': 'error',
        'remediation': 'A negative activity count is physically impossible and points to a broken transform or a bad source row; any metric that sums the column is corrupted. Inspect the stored failing rows by unique_key, trace back to the m365 sharepoint/onedrive feeder mapping, and fix the source. A NULL count (a column a product never emits, e.g. visited_page_count on OneDrive) is not a violation.'
    }
) }}
-- Business-rule data test (#1321 silver-layer integrity).
-- Activity counts can never be negative; a negative value means a broken
-- transform or bad source row and would corrupt any metric that sums them.
-- NULL (not-ingested, e.g. visited_page_count on OneDrive) is intentionally
-- NOT flagged — NULL < 0 is NULL, not a violation; honest NULLs are handled
-- separately. Read FINAL so transient ReplacingMergeTree duplicates can't
-- surface as repeated violations of the same row. `date` is selected purely
-- as triage context in the stored-failure rows (the activity day of the bad
-- count) — it is not a filter: a negative count is corruption on any partition,
-- so the check stays unbounded over the full table.
SELECT
    unique_key,
    date,
    viewed_or_edited_count,
    synced_count,
    shared_internally_count,
    shared_externally_count,
    visited_page_count
FROM {{ ref('class_collab_document_activity') }} FINAL
WHERE viewed_or_edited_count   < 0
   OR synced_count             < 0
   OR shared_internally_count  < 0
   OR shared_externally_count  < 0
   OR visited_page_count       < 0
