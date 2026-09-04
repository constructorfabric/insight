{#-
  Turns one `staging.jira_changelog_items` row into a delta, per the field's
  `field_kind`. See `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md`
  §5.

  The shape of a changelog item is a property of the FIELD, never of the item's
  own content. Deciding it by inspecting the values is what drops every
  labels-type event and mangles every bracketed-id event in the current
  pipeline; these macros are driven by `field_kind` alone.

  Two families:

  * self-describing kinds — every item carries the complete state of both sides,
    so `jira_delta_sides` returns (before_ids, before_displays, after_ids,
    after_displays) and no accumulation is needed;
  * element-wise kinds (`obj_array`) — an item carries one element that was
    added or removed, so `jira_delta_action` and `jira_delta_element` describe
    the change and the caller folds it over the running state.
-#}


{#- ---------- separator helpers ----------

  Each kind's separator is fixed by how Jira renders that kind, not by what the
  value happens to contain:

  * `string_array` — a single space. Jira rejects whitespace inside a label, so
    a space is always a separator and a comma is always content.
  * `option_array` — the id side is a bracketed list separated by `", "`; the
    display side is joined by a bare `,`.
  * `legacy_list` — `", "` on both sides (the pre-2020 Sprint serialization).
-#}

{% macro jira_split_space(s) %}
    arrayFilter(x -> x != '', splitByChar(' ', COALESCE({{ s }}, '')))
{% endmacro %}

{% macro jira_split_comma_space(s) %}
    arrayFilter(x -> x != '', splitByString(', ', COALESCE({{ s }}, '')))
{% endmacro %}

{% macro jira_split_bracketed_ids(s) %}
    arrayMap(x -> trim(BOTH ' ' FROM x),
             arrayFilter(x -> x != '',
                 splitByChar(',', trim(BOTH '[]' FROM COALESCE({{ s }}, '')))))
{% endmacro %}

{% macro jira_split_bare_comma(s) %}
    arrayFilter(x -> x != '', splitByChar(',', COALESCE({{ s }}, '')))
{% endmacro %}


{#- ---------- self-describing kinds ----------

  Returns Tuple(before_ids, before_displays, after_ids, after_displays).

  `scalar`: the id and display sides are not reliably both present. Numeric
  duration fields carry the same text in both; a date carries the machine value
  in the id side and a rendered one in the display side; a summary, a
  description and a story-point estimate carry NOTHING in the id side. Taking
  the id side when present and the display side otherwise is what reconciles all
  four against the issue JSON — and, for a date, it is the id side that matches.

  `issue_ref`: the changelog holds the referenced issue's numeric id while the
  issue JSON holds its key, so the id spaces do not reconcile (§3.1). The key is
  used as both value and identifier on both sides, which keeps the two arrays
  parallel — the class contract requires that, and comparing keys is what makes
  the round-trip check meaningful for this kind.

  `option_array`: the id side is authoritative because the bracketed form parses
  unambiguously. The display side is joined by a bare comma, so a display that
  itself contains a comma would split wrongly; when the two sides disagree on
  length the ids are used as displays rather than emitting a mismatched pair.
-#}
{% macro jira_delta_sides(kind, from_id, from_str, to_id, to_str) %}
{%- set f_ids = jira_split_bracketed_ids(from_id) -%}
{%- set t_ids = jira_split_bracketed_ids(to_id) -%}
{%- set f_disp = jira_split_bare_comma(from_str) -%}
{%- set t_disp = jira_split_bare_comma(to_str) -%}
    multiIf(
        {{ kind }} = 'scalar',
        (
            if(COALESCE({{ from_id }}, {{ from_str }}, '') = '', [], [CAST(COALESCE({{ from_id }}, {{ from_str }}) AS String)]),
            if(COALESCE({{ from_id }}, {{ from_str }}, '') = '', [], [CAST(COALESCE({{ from_id }}, {{ from_str }}) AS String)]),
            if(COALESCE({{ to_id }},   {{ to_str }},   '') = '', [], [CAST(COALESCE({{ to_id }},   {{ to_str }})   AS String)]),
            if(COALESCE({{ to_id }},   {{ to_str }},   '') = '', [], [CAST(COALESCE({{ to_id }},   {{ to_str }})   AS String)])
        ),

        {#- duration: the same two sides as `scalar`, with zero folded to the
            empty state — the changelog cannot tell "estimated zero" from "no
            estimate", and neither can the issue resource (§3.5). -#}
        {{ kind }} = 'duration',
        (
            if(COALESCE({{ from_id }}, {{ from_str }}, '') = ''
               OR toFloat64OrNull(COALESCE({{ from_id }}, {{ from_str }})) = 0, [],
               [CAST(COALESCE({{ from_id }}, {{ from_str }}) AS String)]),
            if(COALESCE({{ from_id }}, {{ from_str }}, '') = ''
               OR toFloat64OrNull(COALESCE({{ from_id }}, {{ from_str }})) = 0, [],
               [CAST(COALESCE({{ from_id }}, {{ from_str }}) AS String)]),
            if(COALESCE({{ to_id }}, {{ to_str }}, '') = ''
               OR toFloat64OrNull(COALESCE({{ to_id }}, {{ to_str }})) = 0, [],
               [CAST(COALESCE({{ to_id }}, {{ to_str }}) AS String)]),
            if(COALESCE({{ to_id }}, {{ to_str }}, '') = ''
               OR toFloat64OrNull(COALESCE({{ to_id }}, {{ to_str }})) = 0, [],
               [CAST(COALESCE({{ to_id }}, {{ to_str }}) AS String)])
        ),

        {#- datetime: the same two sides as `scalar`, reduced to the instant. The
            changelog spells a moment without milliseconds and the issue
            resource with them, so comparing the raw text makes one value look
            like two — see `jira_canonical_instant`. -#}
        {{ kind }} = 'datetime',
        (
            if(COALESCE({{ from_id }}, {{ from_str }}, '') = '', [], [CAST({{ jira_canonical_instant("COALESCE(" ~ from_id ~ ", " ~ from_str ~ ")") }} AS String)]),
            if(COALESCE({{ from_id }}, {{ from_str }}, '') = '', [], [CAST({{ jira_canonical_instant("COALESCE(" ~ from_id ~ ", " ~ from_str ~ ")") }} AS String)]),
            if(COALESCE({{ to_id }},   {{ to_str }},   '') = '', [], [CAST({{ jira_canonical_instant("COALESCE(" ~ to_id ~ ", " ~ to_str ~ ")") }} AS String)]),
            if(COALESCE({{ to_id }},   {{ to_str }},   '') = '', [], [CAST({{ jira_canonical_instant("COALESCE(" ~ to_id ~ ", " ~ to_str ~ ")") }} AS String)])
        ),

        {{ kind }} = 'issue_ref',
        (
            if(COALESCE({{ from_str }}, '') = '', [], [CAST({{ from_str }} AS String)]),
            if(COALESCE({{ from_str }}, '') = '', [], [CAST({{ from_str }} AS String)]),
            if(COALESCE({{ to_str }},   '') = '', [], [CAST({{ to_str }}   AS String)]),
            if(COALESCE({{ to_str }},   '') = '', [], [CAST({{ to_str }}   AS String)])
        ),

        {#- option / user / obj: the id and display sides are separate, and the id
            side can be missing while the display is present — an app field, or a
            value Jira renders without an identifier. Testing each side
            independently then leaves the two arrays different lengths, which
            breaks the parallel-arrays invariant; the display stands in as the
            identifier instead, as it does for the list kinds. -#}
        {{ kind }} IN ('option', 'user', 'obj'),
        (
            if(COALESCE({{ from_id }}, {{ from_str }}, '') = '', [],
               [CAST(COALESCE({{ from_id }}, {{ from_str }}) AS String)]),
            if(COALESCE({{ from_str }}, {{ from_id }}, '') = '', [],
               [CAST(COALESCE({{ from_str }}, {{ from_id }}) AS String)]),
            if(COALESCE({{ to_id }}, {{ to_str }}, '') = '', [],
               [CAST(COALESCE({{ to_id }}, {{ to_str }}) AS String)]),
            if(COALESCE({{ to_str }}, {{ to_id }}, '') = '', [],
               [CAST(COALESCE({{ to_str }}, {{ to_id }}) AS String)])
        ),

        {{ kind }} = 'string_array',
        (
            CAST({{ jira_split_space(from_str) }} AS Array(String)),
            CAST({{ jira_split_space(from_str) }} AS Array(String)),
            CAST({{ jira_split_space(to_str) }}   AS Array(String)),
            CAST({{ jira_split_space(to_str) }}   AS Array(String))
        ),

        {#- legacy_list: the id side is numeric and splits unambiguously; a sprint
            NAME can itself contain `", "`, so splitting the display side by it
            over-splits. When the two sides disagree on length the ids stand in
            as displays rather than emitting a mismatched pair. -#}
        {{ kind }} = 'legacy_list',
        (
            CAST({{ jira_split_comma_space(from_id) }} AS Array(String)),
            CAST(if(length({{ jira_split_comma_space(from_str) }}) = length({{ jira_split_comma_space(from_id) }}),
                    {{ jira_split_comma_space(from_str) }},
                    {{ jira_split_comma_space(from_id) }}) AS Array(String)),
            CAST({{ jira_split_comma_space(to_id) }} AS Array(String)),
            CAST(if(length({{ jira_split_comma_space(to_str) }}) = length({{ jira_split_comma_space(to_id) }}),
                    {{ jira_split_comma_space(to_str) }},
                    {{ jira_split_comma_space(to_id) }}) AS Array(String))
        ),

        {#- long_text: the changelog carries Jira's plain-text rendering on both
            sides, so each side is content-addressed the same way the snapshot is
            (§8). Note the two sides of the PIPELINE differ in form — ADF in the
            issue JSON, a rendering here — which is why this kind is exempt from
            the round-trip invariant. -#}
        {{ kind }} = 'long_text',
        (
            if(COALESCE({{ from_str }}, '') = '', [], [CAST({{ jira_text_id(from_str) }} AS String)]),
            if(COALESCE({{ from_str }}, '') = '', [], [CAST({{ jira_text_prefix(from_str) }} AS String)]),
            if(COALESCE({{ to_str }},   '') = '', [], [CAST({{ jira_text_id(to_str) }} AS String)]),
            if(COALESCE({{ to_str }},   '') = '', [], [CAST({{ jira_text_prefix(to_str) }} AS String)])
        ),

        {{ kind }} = 'option_array',
        (
            CAST(if(length({{ f_ids }}) = 0, {{ f_disp }}, {{ f_ids }}) AS Array(String)),
            CAST(if(length({{ f_ids }}) = 0, {{ f_disp }},
                    if(length({{ f_disp }}) = length({{ f_ids }}), {{ f_disp }}, {{ f_ids }})) AS Array(String)),
            CAST(if(length({{ t_ids }}) = 0, {{ t_disp }}, {{ t_ids }}) AS Array(String)),
            CAST(if(length({{ t_ids }}) = 0, {{ t_disp }},
                    if(length({{ t_disp }}) = length({{ t_ids }}), {{ t_disp }}, {{ t_ids }})) AS Array(String))
        ),

        CAST((CAST([] AS Array(String)), CAST([] AS Array(String)),
              CAST([] AS Array(String)), CAST([] AS Array(String)))
             AS Tuple(Array(String), Array(String), Array(String), Array(String)))
    )
{% endmacro %}


{#- ---------- element-wise kinds ----------

  `obj_array` emits one item per element changed: the element is in the `to`
  side when added and in the `from` side when removed. An item with neither side
  carries no information and is skipped — that is the only genuinely degenerate
  case, as distinct from clearing a field to empty, which is an ordinary removal.
-#}
{% macro jira_delta_action(kind, from_id, from_str, to_id, to_str) %}
    multiIf(
        -- All four sides empty: the item carries no information whatsoever. This
        -- is the ONLY degenerate case, and it is not the same as clearing a
        -- field to empty, where the `from` side still names what was removed.
        COALESCE({{ from_id }}, {{ from_str }}, '') = ''
            AND COALESCE({{ to_id }}, {{ to_str }}, '') = '',            'none',
        {{ kind }} NOT IN {{ jira_element_wise_kinds() }},                'set',
        COALESCE({{ to_id }},   {{ to_str }},   '') != '',               'add',
        COALESCE({{ from_id }}, {{ from_str }}, '') != '',               'remove',
                                                                         'none'
    )
{% endmacro %}


{#- The (id, display) of the element an `obj_array` item adds or removes. -#}
{% macro jira_delta_element(from_id, from_str, to_id, to_str) %}
    if(COALESCE({{ to_id }}, {{ to_str }}, '') != '',
       (CAST(COALESCE({{ to_id }},   {{ to_str }},   '') AS String),
        CAST(COALESCE({{ to_str }},  {{ to_id }},    '') AS String)),
       (CAST(COALESCE({{ from_id }}, {{ from_str }}, '') AS String),
        CAST(COALESCE({{ from_str }},{{ from_id }},  '') AS String)))
{% endmacro %}


{#- Deduplicate an array of `id \x1f display` pairs by the ID, keeping the first
    occurrence. Deduplicating the pairs themselves is not enough: the same
    element can carry different displays on different sides — a version renamed
    between the changelog event and the current snapshot is the common case — and
    two such pairs are distinct strings, so the element would count twice. -#}
{% macro jira_distinct_pairs_by_id(arr) %}
    arrayFilter(
        (p, i) -> i = indexOf(arrayMap(q -> splitByChar('\x1f', q)[1], {{ arr }}),
                              splitByChar('\x1f', p)[1]),
        {{ arr }}, arrayEnumerate({{ arr }}))
{% endmacro %}


{#- ---------- folding an element-wise field's history ----------

  Elements are carried as one `id \x1f display` string so the ids and displays
  cannot drift apart, and an operation is a `(action, element)` tuple.

  Both folds key on the ELEMENT ID, never on the whole pair: a component or a
  version renamed after the event that touched it arrives with one display in
  the changelog and another in the issue JSON, so the two pairs are different
  strings while being the same element.

  Why a fold and not window set arithmetic. The obvious closed form —
  `(initial ∪ additions up to k) \ removals up to k` — is correct only while
  each element is added at most once and removed at most once. An element added,
  removed and ADDED AGAIN stays subtracted forever, because it is in "every
  removal", so the field's newest state loses a value the issue still holds. Set
  arithmetic cannot express a cycle; a sequential fold can, and the per-(issue,
  field) event count is small enough that it costs nothing.
-#}

{#- The pairs of `arr` whose element is not the one `pair` names. -#}
{% macro jira_pairs_without(arr, pair) %}
    arrayFilter(x -> splitByChar('\x1f', x)[1] != splitByChar('\x1f', {{ pair }})[1], {{ arr }})
{% endmacro %}


{#- Apply an ordered operation list forward onto a state.

    An `add` replaces any existing entry for the element rather than appending,
    so a re-add carries the display the later event rendered. -#}
{% macro jira_apply_ops(ops, state) %}
    arrayFold((acc, op) ->
        if(op.1 = 'add',
           arrayPushBack({{ jira_pairs_without('acc', 'op.2') }}, op.2),
           {{ jira_pairs_without('acc', 'op.2') }}),
        {{ ops }}, {{ state }})
{% endmacro %}


{#- The mirror, walked backwards: turns a state into the state BEFORE the list.
    Undoing an `add` drops the element; undoing a `remove` puts it back. -#}
{% macro jira_undo_ops(ops, state) %}
    arrayFold((acc, op) ->
        if(op.1 = 'add',
           {{ jira_pairs_without('acc', 'op.2') }},
           arrayPushBack({{ jira_pairs_without('acc', 'op.2') }}, op.2)),
        reverse({{ ops }}), {{ state }})
{% endmacro %}


{#- Deduplicate a parallel (ids, displays) pair by ID, keeping the first
    occurrence of each id and its display.

    This is applied ONCE, where the journal emits its value arrays — not per
    kind. Doing it per kind missed `option_array` twice: Jira's own bracketed
    list can repeat an id, and an element can carry different displays on
    different sides (a version renamed between the event and the snapshot), so
    deduplicating the pairs rather than the ids leaves the element counted twice.

    `assert_no_duplicate_items_in_array` is what fails when this is skipped. -#}
{% macro jira_distinct_arrays_by_id(ids, displays, want) %}
    arrayMap(i -> {{ 'splitByChar(\'\x1f\', i)[1]' if want == 'ids' else 'splitByChar(\'\x1f\', i)[2]' }},
        {{ jira_distinct_pairs_by_id(
             "arrayMap(j -> concat(" ~ ids ~ "[j], '\x1f', " ~ displays ~ "[j]), range(1, length(" ~ ids ~ ") + 1))") }})
{% endmacro %}
