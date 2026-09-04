{# Join condition matching a merged pull request to the commit its merge
   produced, for the two models that need that commit: git_derived_commits and
   git_superseded_file_changes.

   WORKAROUND: the reported hash is RESOLVED against the collected commit, not
   compared to it. Bitbucket Cloud answers `merge_commit.hash` with a
   12-character prefix where GitHub answers the full 40, so an equality join
   matches no Bitbucket result at all. #3161

   Scoped to the request's own repository: a prefix is a weak key, and a commit
   in another repository that happens to share it is another author's work.
   The caller decides what to do when more than one commit matches — both
   callers refuse to mark anything. #}
{% macro git_merge_result_match(request, commit) -%}
{{ commit }}.tenant_id = {{ request }}.tenant_id
        AND {{ commit }}.source_id = {{ request }}.source_id
        AND {{ commit }}.project_key = {{ request }}.project_key
        AND {{ commit }}.repo_slug = {{ request }}.repo_slug
        AND {{ commit }}.data_source = {{ request }}.data_source
        AND startsWith({{ commit }}.commit_hash, {{ request }}.merge_commit_hash)
{%- endmacro %}
