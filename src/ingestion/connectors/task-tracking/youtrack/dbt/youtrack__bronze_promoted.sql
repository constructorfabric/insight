{{ config(
    materialized='view',
    schema='staging',
    tags=['youtrack']
) }}

{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_projects', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_users', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_custom_fields', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_project_fields', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_field_values', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_agiles', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_sprints', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issue_keys', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issues', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_activities', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_work_items', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_comments', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issue_links', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issue_sprints', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issue_census', order_by='unique_key') %}

SELECT 1 AS promoted
