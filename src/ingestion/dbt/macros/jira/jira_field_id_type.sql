{#-
  What the `value_ids` of a field actually are, as the class contract's
  `value_id_type` enum spells it. Derived from `field_kind`, so it follows the
  same metadata as everything else.

  The Rust binary decides this by matching three field names (`labels`,
  `assignee`, `reporter`) and otherwise falling back to a has-id flag. That gets
  the system fields right and every custom field of the same shape wrong: a
  custom user picker is an account id too, and a custom labels-type field holds
  string literals, not opaque ids.

  `scalar` reports `none`: its id and display are the same text, so there is no
  separate identifier to describe. `issue_ref` reports `string_literal` because
  the value it carries is the referenced issue's key (§3.1).
-#}

{#- The kinds whose changelog emits ONE ELEMENT per item rather than the whole
    list, so their state accumulates and the caller has to fold it. Named once:
    the journal, the delta macros and the classifier all have to agree, and a
    kind added to one and not the others is silently mis-parsed. -#}
{% macro jira_element_wise_kinds() %}('obj_array', 'link_array'){% endmacro %}


{% macro jira_field_id_type(kind) %}
    multiIf(
        {{ kind }} IN ('string_array', 'issue_ref', 'link_array'),   'string_literal',
        {{ kind }} = 'user',                                        'account_id',
        {{ kind }} IN ('option', 'obj', 'obj_array',
                       'option_array', 'legacy_list'),              'opaque_id',
        'none'
    )
{% endmacro %}


{#- The class contract's `field_cardinality` enum. -#}
{% macro jira_field_cardinality(kind) %}
    if({{ kind }} IN ('string_array', 'obj_array', 'option_array', 'legacy_list',
                      'link_array'),
       'multi', 'single')
{% endmacro %}


{#- Where a row of one instant sits relative to the others.

    `synthetic_initial` is the state at issue creation, `changelog` a state after
    an event, `retired_field` the moment the field stopped being returned — so
    that is their order whenever `event_at` ties, and it ties routinely: every
    initial row of an issue carries the creation timestamp, and an issue whose
    first event happened at creation has both kinds on the same instant.

    `_seq` cannot serve here. It is 0 for every changelog row and 1..N for the
    initial rows, which sorts an initial row AFTER an event of the same instant
    — so the newest state of such a field reads as the empty state it had before
    the event. The contract's claim that `(event_at, _seq)` is a total order
    holds only when no two kinds share an instant. -#}
{% macro jira_event_rank(event_kind) %}
    multiIf({{ event_kind }} = 'synthetic_initial', 0,
            {{ event_kind }} = 'changelog',         1,
                                                   2)
{% endmacro %}
