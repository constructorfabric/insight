{% macro iso8601_duration_seconds(col) %}
{#- Parse ISO 8601 duration (PT1H30M45S or PT1M30.5S) to seconds. CH 25.x compatible. -#}
{#- Seconds may be fractional (Graph API returns e.g. PT1M30.5S), so parse as Float64. -#}
toInt64(
    coalesce(nullIf(extractAll({{ col }}, '(\d+)H')[1], ''), '0')
) * 3600
+ toInt64(
    coalesce(nullIf(extractAll({{ col }}, '(\d+)M')[1], ''), '0')
) * 60
+ toFloat64OrZero(
    coalesce(nullIf(extractAll({{ col }}, '(\d+(?:\.\d+)?)S')[1], ''), '0')
)
{% endmacro %}
