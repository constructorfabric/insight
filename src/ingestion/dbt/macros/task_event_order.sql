{#-
  `event_order` of `silver.class_task_field_history`: the one column that orders
  a task's history, within one field and across all fields of the task.

      event_order = ms(order_at) * 1 000 000 + band * 100 000 + rank

  `order_at` is `event_at`, except on `synthetic_initial` rows, where it is the
  earlier of the issue's creation and its first recorded event: an imported
  history can date an event before the creation, and the band decides only
  within one millisecond.

  `band` is the kind (`task_event_band`), `rank` the row's position within its
  (task, instant, band): for a changelog row the position of its ENTRY among the
  task's entries of that instant, shared by every field the entry changed; for a
  `synthetic_initial` row the field's index, the creation marker taking 0.

  INVARIANT: `rank` < 100 000 and `band` < 10, or a row leaks into the next
  band or millisecond. Six extra digits keep the value inside Int64 until well
  past the year 2200.
-#}
{% macro task_event_order(order_at, band, rank) -%}
    (toUnixTimestamp64Milli(toDateTime64({{ order_at }}, 3)) * 1000000
     + toInt64({{ band }}) * 100000
     + toInt64({{ rank }}))
{%- endmacro %}

{#-
  The kind's band within one millisecond: the state at creation precedes any
  event, and rows observed rather than recorded (`retired_field`,
  `snapshot_diff`, `unclassified_field`, `availability`, `lifecycle`) follow
  the events they accompany.
-#}
{% macro task_event_band(event_kind) -%}
    multiIf({{ event_kind }} = 'synthetic_initial', 0,
            {{ event_kind }} = 'changelog',         1,
                                                   2)
{%- endmacro %}
