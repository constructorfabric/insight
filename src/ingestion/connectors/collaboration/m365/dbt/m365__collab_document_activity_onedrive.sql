{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['m365', 'silver:class_collab_document_activity']
) }}

-- OneDrive half of document activity.
-- Split from SharePoint (see m365__collab_document_activity_sharepoint) so each
-- product has its own incremental watermark. Unioned at silver by tag.

WITH newest_extract AS (

    SELECT
        tenant_id,
        source_id AS insight_source_id,
        MD5(concat(tenant_id, '-', source_id, '-', lower(coalesce(userPrincipalName, '')), '-', toString(reportRefreshDate), '-', 'onedrive')) AS unique_key,
        lower(userPrincipalName) AS user_id,
        lower(userPrincipalName) AS user_name,
        lower(userPrincipalName) AS email,
        if(userPrincipalName IS NOT NULL AND userPrincipalName != '',
           lower(userPrincipalName),
           '') AS person_key,
        toDate(reportRefreshDate) AS date,
        'onedrive' AS product,
        viewedOrEditedFileCount AS viewed_or_edited_count,
        syncedFileCount AS synced_count,
        sharedInternallyFileCount AS shared_internally_count,
        sharedExternallyFileCount AS shared_externally_count,
        CAST(NULL AS Nullable(Int64)) AS visited_page_count,
        reportPeriod AS report_period,
        now() AS collected_at,
        'insight_m365' AS data_source,
        toUnixTimestamp64Milli(now64()) AS _version,
        (assignedProducts IS NULL OR replaceRegexpAll(assignedProducts, '[[:space:]]', '') NOT IN ('', '[]')) AS licensed
    FROM {{ source('bronze_m365', 'onedrive_activity') }} FINAL
    WHERE userPrincipalName IS NOT NULL
      AND userPrincipalName != ''
      AND {{ m365_day_was_reported('onedrive_activity') }}
    {% if is_incremental() %}
      -- Watermark on the source EXTRACT time, not the business date (see zoom model header
      -- for the backfill-strand failure mode this fixes). Re-pulled rows carry a fresh
      -- `_airbyte_extracted_at`, so reprocess every business date touched by a recent extract.
      AND (
        (SELECT count() FROM {{ this }}) = 0
        OR toDate(reportRefreshDate) IN (
          SELECT DISTINCT toDate(reportRefreshDate)
          FROM {{ source('bronze_m365', 'onedrive_activity') }}
          WHERE _airbyte_extracted_at
                > (SELECT max(_airbyte_extracted_at) FROM {{ source('bronze_m365', 'onedrive_activity') }}) - INTERVAL 3 DAY
        )
      )
    {% endif %}
    -- INVARIANT: Microsoft may return one account's UPN in a different letter case
    -- on another extract; the key is case-insensitive and the newest extract wins.
    -- The license filter runs after this pick, so an older licensed variant never
    -- stands in for a newer unlicensed one.
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key

)

SELECT * EXCEPT (licensed)
FROM newest_extract
-- Drop unlicensed users (see #736 / teams feeder). This MS Graph report
-- (getOneDriveActivityUserDetail) exposes no `isLicensed`; the only license signal
-- is `assignedProducts` — the products assigned to the user, empty for unlicensed
-- accounts. Airbyte stores the array as a JSON string, so an empty list is '[]'.
-- Conservative on NULL/unknown (keep); drop only explicitly-empty product lists.
WHERE licensed
