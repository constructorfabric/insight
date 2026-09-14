{#-
  Rank of an event KIND within one instant of one (issue, field), for the
  ordering key of `silver.class_task_field_history`.

  `event_at` alone is not a total order, and neither is `(event_at, _seq)`: a
  changelog row carries `_seq` 0 while the `synthetic_initial` rows of an issue
  carry 1..N, so ordering by `_seq` sorts the initial row AFTER an event of the
  same instant — and the newest state of that field then reads as the state
  before the event. Every issue whose first event landed on its own creation
  timestamp hits this.

  The kind decides instead: an initial row is by definition the state before any
  event; a `changelog` row is a state after one; `availability` and `lifecycle`
  are observed at or after the events they accompany, so they sort last.

  The full key is `(event_at, task_event_rank(event_kind), _seq,
  toUInt64OrZero(event_id))` — `_seq` orders the initial rows among themselves
  (they share the creation timestamp) and the event id breaks a tie between two
  changelog rows of one instant, compared NUMERICALLY because as text '101'
  sorts before '99'.
-#}
{% macro task_event_rank(event_kind) %}
    multiIf({{ event_kind }} = 'synthetic_initial', 0,
            {{ event_kind }} = 'changelog',         1,
                                                   2)
{% endmacro %}
