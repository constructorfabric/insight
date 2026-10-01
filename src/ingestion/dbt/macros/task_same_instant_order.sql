{#-
  The order of a task's changelog entries that share one instant, proved from
  the entries themselves.

  Each field an entry changed states where its value went from and to. Among the
  entries of one instant that touched a field, a unique from→to chain is the
  only order the field's values allow; `task_chain_walk` finds it. Chains of all
  fields together link the entries (`task_instant_order`), and the numeric
  changelog id decides only what no link does.

  WORKAROUND: ClickHouse lambdas have no `let`, and a sub-expression spelled out
  at every use multiplies the query's AST until the analyzer stalls. Each value
  below is bound once, as the argument of a one-element arrayMap.
-#}

{% macro _task_let(name, value, body) -%}
    arrayMap({{ name }} -> {{ body }}, [{{ value }}])[1]
{%- endmacro %}

{#-
  The unique from→to chain through one field's events of one instant, as
  1-based positions into `befores` / `afters` in chain order, or [] when there
  is none: a fork, a cycle, a repeated side, or an entry carrying two items of
  the field (`ambiguous`).

  INVARIANT: a walk that leaves the chain reads index 0, which the final check
  rejects.
-#}
{% macro task_chain_walk(befores, afters, ambiguous) -%}
    {%- set check -%}
        if(NOT ({{ ambiguous }})
             AND length(arrayDistinct(chain_befores)) = length(chain_befores)
             AND length(arrayDistinct(chain_afters)) = length(chain_befores)
             AND NOT arrayExists(p -> chain_nexts[p] = p, arrayEnumerate(chain_befores))
             AND length(chain_heads) = 1
             AND NOT has(chain_walk, 0)
             AND length(arrayDistinct(chain_walk)) = length(chain_befores),
           chain_walk,
           CAST([] AS Array(UInt64)))
    {%- endset -%}
    {%- set walk = "arrayFold((acc, k) -> arrayPushBack(acc, chain_nexts[acc[-1]]), range(1, length(chain_befores)), [toUInt64(chain_heads[1])])" -%}
    {%- set heads = "arrayFilter(p -> NOT has(chain_nexts, p), arrayEnumerate(chain_befores))" -%}
    {%- set nexts = "arrayMap(a -> toUInt64(indexOf(chain_befores, a)), chain_afters)" -%}
    {{ _task_let('chain_befores', befores,
         _task_let('chain_afters', afters,
           _task_let('chain_nexts', nexts,
             _task_let('chain_heads', heads,
               _task_let('chain_walk', walk, check))))) }}
{%- endmacro %}

{#-
  The entries of one instant in order, as a permutation of 1..`n`: `edges` holds
  (earlier, later) pairs of 1-based entry positions, and the entries are
  numbered by changelog id, so a lower position is the fallback.

  Links that contradict each other are dropped, and only those: an edge whose
  later entry can reach its earlier one lies on a cycle, and every entry on it
  falls back to the id while the links around it still hold. The rest is the
  smallest topological order.

  INVARIANT: reachability costs cubic time in the entries left on cycles, so
  above `cycle_cap` of them every link among them is dropped instead.
-#}
{% macro task_instant_order(n, edges, cycle_cap=64) -%}
    {%- set second_pass = _task_let('order_edges', _task_acyclic_edges('order_first', cycle_cap),
                                    _task_topological_order()) -%}
    {{ _task_let('order_n', 'toUInt64(' ~ n ~ ')',
         _task_let('order_edges', edges,
           _task_let('order_first', _task_topological_order(),
             'if(NOT has(order_first, toUInt64(0)), order_first, ' ~ second_pass ~ ')'))) }}
{%- endmacro %}

{#-
  Kahn's algorithm over `order_n` positions and `order_edges`, always taking the
  lowest position free of predecessors. A step with none left (a cycle) places 0.
-#}
{% macro _task_topological_order() -%}
    {%- set step -%}
        (
            arrayPushBack(acc.1, pick),
            if(pick = 0,
               acc.2,
               arrayMap((d, p) -> if(p = pick, toInt64(-1), d - toInt64(countEqual(acc.3[pick], p))),
                        acc.2, range(1, order_n + 1))),
            acc.3
        )
    {%- endset -%}
    arrayFold(
        (acc, s) -> {{ _task_let('pick', 'toUInt64(arrayFirstIndex(x -> x = 0, acc.2))', step) }},
        range(1, order_n + 1),
        (CAST([] AS Array(UInt64)),
         arrayMap(p -> toInt64(countEqual(arrayMap(e -> e.2, order_edges), p)), range(1, order_n + 1)),
         arrayMap(p -> arrayMap(e -> e.2, arrayFilter(e -> e.1 = p, order_edges)), range(1, order_n + 1)))
    ).1
{%- endmacro %}

{#-
  `order_edges` without those lying on a cycle among the entries `placed` missed.
-#}
{% macro _task_acyclic_edges(placed, cycle_cap) -%}
    {%- set keep = "arrayFilter(e -> NOT (has(stuck, e.1) AND has(stuck, e.2) AND (length(stuck) > " ~ cycle_cap ~ " OR has(reach[e.2], e.1))), order_edges)" -%}
    {%- set reach = "arrayFold((r, s) -> arrayMap(p -> arrayDistinct(arrayConcat(r[p], arrayFlatten(arrayMap(q -> r[q], r[p])))), range(1, order_n + 1)), range(if(length(stuck) > " ~ cycle_cap ~ ", 0, toUInt64(ceil(log2(order_n))) + 1)), arrayMap(p -> arrayMap(e -> e.2, arrayFilter(e -> e.1 = p AND has(stuck, e.1) AND has(stuck, e.2), order_edges)), range(1, order_n + 1)))" -%}
    {{ _task_let('stuck', 'arrayFilter(p -> NOT has(' ~ placed ~ ', p), range(1, order_n + 1))',
         _task_let('reach', reach, keep)) }}
{%- endmacro %}
