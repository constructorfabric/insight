{# The identity of one file change's CONTENT: the object id it produced.

   Keyed on the post-image alone rather than the (pre, post) pair, because a
   squash re-applies a branch's whole span as one step — the branch makes
   A to B then B to C, the squash makes A to C — and a pair key reads that as a
   third, unseen change. The resulting content is what the two lines of history
   share, so it is what identifies the work.

   A deletion produces no content, so its identity is what it REMOVED: without
   the pre-image the key would be constant and every deletion at one path would
   collapse into one, whatever content each removed.

   Empty on both sides means the source reports no object id at all. The caller
   keeps those rows distinct per commit — see the tie-breaker in
   `deduplicated_file_changes`. #}
{% macro git_file_content_identity(post_image_oid, pre_image_oid) -%}
if(
        coalesce({{ post_image_oid }}, '') != '',
        {{ post_image_oid }},
        concat('~removed~', coalesce({{ pre_image_oid }}, ''))
    )
{%- endmacro %}
