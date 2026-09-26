{#-
  Rank of an event KIND within one instant of one (issue, field), for the
  ordering key of `silver.class_task_field_history`.

  `event_at` alone is not a total order, and neither is `(event_at, _seq)`: a
  Jira changelog row's `_seq` is its position in the from→to chain of events
  sharing its instant (0 when that chain is not unique), an element-wise
  changelog row's `_seq` is always 0, and the `synthetic_initial` rows of an
  issue carry 1..N — so ordering by `_seq` alone can still sort an initial row
  AFTER an event of the same instant, reading the newest state of that field as
  the state before the event. Every issue whose first event landed on its own
  creation timestamp hits this.

  The kind decides instead: an initial row is by definition the state before any
  event; a `changelog` row is a state after one; `availability` and `lifecycle`
  are observed at or after the events they accompany, so they sort last.

  The full key is `(event_at, task_event_rank(event_kind), _seq,
  toUInt64OrZero(event_id))` — `_seq` orders the initial rows among themselves
  and a self-describing changelog row's own chain among its ties; the event id
  breaks whatever `_seq` still leaves tied, compared NUMERICALLY because as
  text '101' sorts before '99'.
-#}
{% macro task_event_rank(event_kind) %}
    multiIf({{ event_kind }} = 'synthetic_initial', 0,
            {{ event_kind }} = 'changelog',         1,
                                                   2)
{% endmacro %}
