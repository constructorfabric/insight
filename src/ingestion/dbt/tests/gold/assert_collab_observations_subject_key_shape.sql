-- Build-integrity check (untagged → error severity under `dbt build`).
-- subject_key is the per-subject contract: present on every row of the
-- distinct-count measures (active_day, active_modality, chat_active_day) and of
-- meetings_attended (the meeting, or the address of a report that names none),
-- and absent everywhere else. A violation means a measure branch landed in the
-- wrong UNION arm of the final projection.
SELECT
    measure_key,
    countIf(subject_key IS NULL) AS null_subject_rows,
    countIf(subject_key IS NOT NULL) AS subject_rows
FROM {{ ref('collab_metric_observations') }}
GROUP BY measure_key
HAVING (measure_key IN ('active_day', 'active_modality', 'chat_active_day', 'meetings_attended') AND null_subject_rows > 0)
    OR (measure_key NOT IN ('active_day', 'active_modality', 'chat_active_day', 'meetings_attended') AND subject_rows > 0)
