{#-
  The grain of a task measure that is not additive across accounts — a gate on
  another measure, a deviation from a ratio: the person-day. Gold cannot see the
  person, so the email stands for it where the tracker states one, and an
  email-less account stands for itself. The account key survives only on a day
  one account made alone, so that row still resolves account-first; a day several
  accounts of one email made resolves through the email.

  INVARIANT: the input columns carry a prefix the output aliases do not, so the
  GROUP BY never reads an aggregate alias back.
-#}

{% macro task_person_day_group(prefix) %}
    if(
        {{ prefix }}entity_id = '',
        concat(
            coalesce({{ prefix }}account_source_type, ''), ':',
            coalesce({{ prefix }}account_source_id, ''), ':',
            coalesce({{ prefix }}account_id, '')
        ),
        ''
    )
{% endmacro %}

{% macro task_person_day_account_columns(prefix) %}
    {% for column in ['account_source_type', 'account_source_id', 'account_id'] %}
    if(
        uniqExact(
            coalesce({{ prefix }}account_source_type, ''),
            coalesce({{ prefix }}account_source_id, ''),
            coalesce({{ prefix }}account_id, '')
        ) = 1,
        any(coalesce({{ prefix }}{{ column }}, '')),
        ''
    ) AS {{ column }}{% if not loop.last %},{% endif %}
    {% endfor %}
{% endmacro %}
