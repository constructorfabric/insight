{#-
  Normalizes a raw JSON fragment from `bronze_jira.jira_issue.custom_fields_json`
  into the `(value_ids, value_displays)` pair the field-history contract stores.
  See `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md` §4.

  All macros return a `Tuple(Array(String), Array(String))` expression, so a
  caller reads `.1` for ids and `.2` for displays. An absent or null fragment
  yields two empty arrays — the "applicable but not filled" state, distinct from
  a key that is absent from the issue JSON altogether (§6), which emits no row.

  Ids arrive both quoted and unquoted in the same position across fields: an
  option id is `"19272"` while a sprint id is `2151`. Every id and scalar
  therefore goes through `jira_json_unquote`, never through `JSONExtractString`
  alone, which returns '' for an unquoted number.
-#}


{#- Raw JSON scalar -> plain text. Handles quoted strings, bare numbers and
    booleans; empty for an absent or null fragment. -#}
{% macro jira_json_unquote(v) %}
    if({{ v }} IN ('', 'null'), '',
       if(startsWith({{ v }}, '"'), JSONExtractString({{ v }}), {{ v }}))
{% endmacro %}


{#- The id of a single JSON object. Jira spells it `id` on options, versions,
    components, statuses, projects and issues, and `accountId` on users. -#}
{% macro jira_json_obj_id(v) %}
    coalesce(
        nullIf({{ jira_json_unquote("JSONExtractRaw(" ~ v ~ ", 'id')") }}, ''),
        nullIf({{ jira_json_unquote("JSONExtractRaw(" ~ v ~ ", 'accountId')") }}, ''),
        ''
    )
{% endmacro %}


{#- The human-readable side of a single JSON object.
    Probe order is load-bearing, not arbitrary:
      `value`       options and multi-selects;
      `name`        versions, components, sprints, statuses, resolutions, and
                    projects — a project carries BOTH `name` and `key`, and the
                    changelog renders its name, so `name` must win;
      `displayName` users;
      `filename`    attachments — without it an attachment's display is empty in
                    the snapshot while the changelog supplies a name, so the two
                    disagree and the same element counts twice;
      `key`         issue references (a parent has `key` and no `name`).
    -#}
{% macro jira_json_obj_display(v) %}
    coalesce(
        nullIf(JSONExtractString({{ v }}, 'value'), ''),
        nullIf(JSONExtractString({{ v }}, 'name'), ''),
        nullIf(JSONExtractString({{ v }}, 'displayName'), ''),
        nullIf(JSONExtractString({{ v }}, 'filename'), ''),
        nullIf(JSONExtractString({{ v }}, 'key'), ''),
        ''
    )
{% endmacro %}


{#- kind = scalar: a bare string, number or date. Id and display are the same
    text — there is no separate identifier. A datetime has its own kind. -#}
{% macro jira_norm_scalar(v) %}
    (
        if({{ v }} IN ('', 'null'), [], [CAST({{ jira_json_unquote(v) }} AS String)]),
        if({{ v }} IN ('', 'null'), [], [CAST({{ jira_json_unquote(v) }} AS String)])
    )
{% endmacro %}


{#- kind = duration: a time-tracking estimate, where zero is not a value.

    Jira renders "no remaining estimate" as `0` or as `null` depending on
    whether the time-tracking container is populated, and its changelog writes
    the literal `0` for both — including for `null -> 0`, which is what logging
    work against an unestimated issue emits and cannot be a deliberate zero.
    A cleared estimate (`28800 -> 0`) also comes back as `null`.

    So neither side can express "the estimate is zero" as distinct from "there
    is no estimate", and both spellings are folded to the empty state. Nothing a
    consumer can use is lost: whether an issue was ever estimated is answered by
    the field having a non-zero history, not by zero-versus-absent. §3.5. -#}
{% macro jira_norm_duration(v) %}
{%- set text = jira_json_unquote(v) -%}
    (
        if({{ v }} IN ('', 'null') OR toFloat64OrNull({{ text }}) = 0, [],
           [CAST({{ text }} AS String)]),
        if({{ v }} IN ('', 'null') OR toFloat64OrNull({{ text }}) = 0, [],
           [CAST({{ text }} AS String)])
    )
{% endmacro %}


{#- A datetime rendered to one canonical instant.

    The two sides of the pipeline spell the same moment differently: the issue
    resource writes `2026-01-05T07:00:00.000+0000` and the changelog
    `2026-01-05T07:00:00+0000`. Compared as text those are two values, so the
    round-trip invariant reports every datetime field an issue carries. They are
    one instant, and parsing both to it is what makes them one value again.

    An offset other than UTC folds to the same instant rather than to the same
    text, which is the point. A value that does not parse is passed through
    unchanged: an unrecognised spelling must stay visible as itself, never be
    silently dropped. -#}
{% macro jira_canonical_instant(t) %}
    COALESCE(toString(parseDateTime64BestEffortOrNull(CAST({{ t }} AS String), 3, 'UTC')),
             CAST({{ t }} AS String))
{% endmacro %}


{#- kind = datetime: like `scalar`, but both sides reduced to the instant. -#}
{% macro jira_norm_datetime(v) %}
    (
        if({{ v }} IN ('', 'null'), [],
           [CAST({{ jira_canonical_instant(jira_json_unquote(v)) }} AS String)]),
        if({{ v }} IN ('', 'null'), [],
           [CAST({{ jira_canonical_instant(jira_json_unquote(v)) }} AS String)])
    )
{% endmacro %}


{#- kinds obj / option / user: one object, one (id, display) pair. -#}
{% macro jira_norm_single_obj(v) %}
    (
        if({{ v }} IN ('', 'null'), [], [CAST({{ jira_json_obj_id(v) }} AS String)]),
        if({{ v }} IN ('', 'null'), [], [CAST({{ jira_json_obj_display(v) }} AS String)])
    )
{% endmacro %}


{#- kind = issue_ref: the issue JSON holds the referenced issue's KEY as a bare
    string, while the changelog holds its numeric id with the key as the display.
    The two id spaces do not reconcile, so the KEY serves as both value and
    identifier and the numeric id is discarded — it stays recoverable from
    `bronze_jira.jira_issue_keys`. Emitting an empty id array instead would break
    the parallel-arrays invariant that `assert_value_arrays_same_length` enforces
    on the class contract. -#}
{% macro jira_norm_issue_ref(v) %}
    (
        if({{ v }} IN ('', 'null'), [], [CAST({{ jira_json_unquote(v) }} AS String)]),
        if({{ v }} IN ('', 'null'), [], [CAST({{ jira_json_unquote(v) }} AS String)])
    )
{% endmacro %}


{#- kind = string_array: an array of bare strings, ids and displays identical. -#}
{% macro jira_norm_string_array(v) %}
    (
        CAST(JSONExtract(if({{ v }} IN ('', 'null'), '[]', {{ v }}), 'Array(String)') AS Array(String)),
        CAST(JSONExtract(if({{ v }} IN ('', 'null'), '[]', {{ v }}), 'Array(String)') AS Array(String))
    )
{% endmacro %}


{#- kinds obj_array / option_array / legacy_list: an array of objects. The
    element shape differs by `schema_items` — `{id,value}` for options,
    `{id,name}` for versions, components and sprints, `{accountId,displayName}`
    for users — so element extraction coalesces over those spellings rather than
    branching per kind. The kinds differ in how their DELTAS apply, not in how
    their values are read. -#}
{% macro jira_norm_obj_array(v) %}
    (
        CAST(arrayMap(x -> {{ jira_json_obj_id('x') }},
             JSONExtractArrayRaw(if({{ v }} IN ('', 'null'), '[]', {{ v }}))) AS Array(String)),
        CAST(arrayMap(x -> {{ jira_json_obj_display('x') }},
             JSONExtractArrayRaw(if({{ v }} IN ('', 'null'), '[]', {{ v }}))) AS Array(String))
    )
{% endmacro %}


{#- The linked issue's key, from a link element of the issue resource.

    A link element is `{"id": <the link's own id>, "outwardIssue"|"inwardIssue":
    {"key": ...}}`. The link's own id is what `jira_json_obj_id` would take, and
    it appears nowhere in the changelog — which names the linked issue by key.
    So the key is the identifier on both sides, and the link id is discarded; it
    is recoverable from the issue JSON.

    A SUBTASK element is the referenced issue itself, with the key at the top
    level, so the same rule applies one level shallower.

    Direction is not part of the identity. An issue holds each link once, and
    which side of it the issue is on is a property of the link, not of the
    element — the changelog's rendered text carries it ("blocks", "is caused
    by"), which is why the display keeps that text where it has it. -#}
{% macro jira_json_link_key(v) %}
    coalesce(
        nullIf(JSONExtractString({{ v }}, 'outwardIssue', 'key'), ''),
        nullIf(JSONExtractString({{ v }}, 'inwardIssue', 'key'), ''),
        {#- `subtasks` carries the same `schema_items` as `issuelinks` and so the
            same kind, but its element IS the referenced issue rather than a link
            to it: the key sits at the top level and there is no link object. A
            link element never has a top-level `key`, so probing it last is
            unambiguous. Without this the whole field normalizes to empty
            strings — parallel arrays of nothing, which no test but the round
            trip would notice. -#}
        nullIf(JSONExtractString({{ v }}, 'key'), ''),
        ''
    )
{% endmacro %}


{#- kind = link_array: the linked issue key as both value and identifier, which
    is what makes the two sides reconcile at all. -#}
{% macro jira_norm_link_array(v) %}
    (
        CAST(arrayMap(x -> {{ jira_json_link_key('x') }},
             JSONExtractArrayRaw(if({{ v }} IN ('', 'null'), '[]', {{ v }}))) AS Array(String)),
        CAST(arrayMap(x -> {{ jira_json_link_key('x') }},
             JSONExtractArrayRaw(if({{ v }} IN ('', 'null'), '[]', {{ v }}))) AS Array(String))
    )
{% endmacro %}


{#- Dispatch on `field_kind`. `ignored` never reaches here. -#}
{% macro jira_norm_value(kind, v) %}
    multiIf(
        {{ kind }} = 'scalar',       {{ jira_norm_scalar(v) }},
        {{ kind }} = 'datetime',     {{ jira_norm_datetime(v) }},
        {{ kind }} = 'duration',     {{ jira_norm_duration(v) }},
        {{ kind }} = 'option',       {{ jira_norm_single_obj(v) }},
        {{ kind }} = 'user',         {{ jira_norm_single_obj(v) }},
        {{ kind }} = 'obj',          {{ jira_norm_single_obj(v) }},
        {{ kind }} = 'issue_ref',    {{ jira_norm_issue_ref(v) }},
        {{ kind }} = 'string_array', {{ jira_norm_string_array(v) }},
        {{ kind }} = 'obj_array',    {{ jira_norm_obj_array(v) }},
        {{ kind }} = 'link_array',   {{ jira_norm_link_array(v) }},
        {{ kind }} = 'option_array', {{ jira_norm_obj_array(v) }},
        {{ kind }} = 'legacy_list',  {{ jira_norm_obj_array(v) }},
        {{ kind }} = 'long_text',    {{ jira_norm_long_text(v) }},
        CAST((CAST([] AS Array(String)), CAST([] AS Array(String)))
             AS Tuple(Array(String), Array(String)))
    )
{% endmacro %}


{#- Content address for a long-text body (§8). A pure function of the text, so
    the same body always resolves to the same `jira__task_field_text` row.
    `sipHash128` is not cryptographic; it is a fast, well-distributed 128-bit
    digest, which is what content addressing needs. -#}
{% macro jira_text_id(content) %}
    lower(hex(sipHash128({{ content }})))
{% endmacro %}


{#- The readable prefix of a long-text body.

    `substringUTF8`, never `substring`: ClickHouse's `substring` counts BYTES,
    so cutting a body at 200 bytes splits whatever multi-byte character spans
    that boundary and the row carries invalid UTF-8. Nothing in the dbt chain
    notices — ClickHouse stores the bytes happily and every array-shape test
    passes — but a consumer that decodes strings strictly rejects the row. The
    Rust reader does exactly that, and dying on "incomplete utf-8 byte sequence
    from index 198" is how this was found.

    The prefix is therefore measured in CHARACTERS, which is also what the
    variable has always been named. -#}
{% macro jira_text_prefix(v) %}
    substringUTF8({{ v }}, 1, {{ var('jira_long_text_prefix_chars', 200) }})
{% endmacro %}


{#- kind = long_text: the journal carries the body's content address plus a short
    prefix, never the body itself — see `jira__task_field_text`. The prefix keeps
    a row readable in a drilldown without pulling kilobytes through the array
    columns on every read of any field.

    `value_id_type` stays `string_literal` rather than gaining a `text_ref` enum
    value: the enum is a shared cross-source contract, and widening it for one
    Jira kind is disproportionate while nothing outside the tests reads it. A
    consumer that needs to tell them apart reads `field_kind`. -#}
{% macro jira_norm_long_text(v) %}
    (
        if({{ v }} IN ('', 'null', '""', '{}'), [],
           [CAST({{ jira_text_id(v) }} AS String)]),
        if({{ v }} IN ('', 'null', '""', '{}'), [],
           [CAST({{ jira_text_prefix(v) }} AS String)])
    )
{% endmacro %}
