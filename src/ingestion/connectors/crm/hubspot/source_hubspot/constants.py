"""HubSpot stream registry, property-type mapping, and API limits."""

from typing import FrozenSet, Mapping

# ------- API -----------------------------------------------------------------

BASE_URL = "https://api.hubapi.com"

# ------- Search endpoint caps ------------------------------------------------

# HubSpot Search returns HTTP 400 once `after >= SEARCH_AFTER_HARD_CAP`.
# Hit this, switch to keyset pagination within the same time slice.
SEARCH_AFTER_HARD_CAP = 10_000

# Search endpoint: 100 records per page is the maximum the API accepts.
SEARCH_PAGE_LIMIT = 100

# v3 list endpoints: 100 records per page (same cap as search).
LIST_PAGE_LIMIT = 100

# v3 batch_read endpoint: max 100 inputs per call.
# Used by archived streams to fetch full property values without the URL
# 414 that would hit on a GET with all properties in the query string.
BATCH_READ_LIMIT = 100

# v4 associations batch_read accepts up to 1000 ids per call, but 100 keeps
# request bodies small enough that a 429 retry doesn't replay a big payload.
ASSOCIATIONS_BATCH_SIZE = 100

# ------- Property-type mapping (describe -> JSON schema) ---------------------

# HubSpot property type -> (json-schema type, optional format).
# Any type not listed falls back to string with a one-time warning.
HUBSPOT_TYPE_TO_JSON_SCHEMA: Mapping[str, tuple] = {
    # HubSpot's CRM v3 Search API returns every property value as a JSON
    # string — booleans come back as "true"/"false", numbers as "1234.56",
    # datetimes as ISO strings (sometimes epoch-millis strings on legacy
    # properties). Declaring anything other than ("string", None) makes
    # the destination build a typed column (Bool/Decimal/DateTime64) and
    # silently NULL every row whose value can't deserialize, with
    # _airbyte_meta.changes recording DESTINATION_SERIALIZATION_ERROR.
    # Observed losses: ~100% on deals.hs_is_closed/won, all values on
    # companies.numberofemployees range strings ("500-1000"), tasks
    # legacy completion dates, etc.
    #
    # Bronze stays as Nullable(String) for every property; dbt coerces
    # downstream (toInt64OrNull, toFloat64OrNull,
    # parseDateTime64BestEffortOrNull). Lossless storage,
    # parser-failure isolation per row instead of silent NULL.
    "string": ("string", None),
    "bool": ("string", None),
    "boolean": ("string", None),
    "enumeration": ("string", None),
    "date": ("string", None),
    "datetime": ("string", None),
    "date-time": ("string", None),
    "number": ("string", None),
    "json": ("string", None),
    "object_coordinates": ("string", None),
    "phone_number": ("string", None),
}

# ------- Cloudflare oddity ---------------------------------------------------

# HubSpot fronts the API via Cloudflare; an invalid token format (e.g. wrong
# prefix) bubbles up as a 530, not a proper 401. Map it to a config error with
# a token-format hint.
CLOUDFLARE_ORIGIN_DNS_ERROR = 530

# ------- Curated stream registry ---------------------------------------------

# ``object_type`` is the path segment used with /crm/v3/objects/{object_type}
# and /crm/v3/objects/{object_type}/search. ``primary_key`` is always "id".
# ``associations`` lists object types to co-fetch via v4 batch_read.
# ``archived_supported`` toggles whether HubSpot exposes archived listing for
# this object type — when False, no ``{name}_archived`` stream is derived.
# Meetings (object 0-47) returns HTTP 400 "Paging through deleted objects is
# not yet supported" so it has no archived sibling.
_LIVE_STREAM_REGISTRY: Mapping[str, Mapping] = {
    "contacts": {
        "object_type": "contacts",
        "cursor_field": "updatedAt",
        # Contacts use ``lastmodifieddate`` (no ``hs_`` prefix); every other
        # CRM object uses ``hs_lastmodifieddate``.
        "search_cursor_property": "lastmodifieddate",
        "associations": ["companies", "deals"],
        "silver_tag": "silver:class_crm_contacts",
        "archived_supported": True,
    },
    "companies": {
        "object_type": "companies",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": [],
        "silver_tag": "silver:class_crm_accounts",
        "archived_supported": True,
    },
    "deals": {
        "object_type": "deals",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["companies", "contacts"],
        "silver_tag": "silver:class_crm_deals",
        "archived_supported": True,
    },
    "engagements_calls": {
        "object_type": "calls",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        "archived_supported": True,
    },
    "engagements_emails": {
        "object_type": "emails",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        "archived_supported": True,
    },
    "engagements_meetings": {
        "object_type": "meetings",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        # HubSpot returns HTTP 400 on /crm/v3/objects/meetings?archived=true
        # ("Paging through deleted objects is not yet supported").
        "archived_supported": False,
    },
    "engagements_tasks": {
        "object_type": "tasks",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        "archived_supported": True,
    },
    "leads": {
        "object_type": "leads",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies"],
        "silver_tag": None,  # bronze-only in v1
        "archived_supported": True,
    },
    "tickets": {
        "object_type": "tickets",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": None,  # bronze-only in v1
        "archived_supported": True,
    },
    # owners is NOT a CRM object — different endpoint shape (/crm/v3/owners).
    # Handled by a dedicated stream class; no search endpoint, no custom
    # properties. The owners list endpoint doesn't accept an updatedAt
    # filter, so the stream pages the full owner set every sync but filters
    # records client-side by ``updatedAt > state``; only changed owners are
    # emitted to the destination after the first sync.
    "owners": {
        "object_type": "owners",
        "cursor_field": "updatedAt",
        "search_cursor_property": None,
        "associations": [],
        "silver_tag": "silver:class_crm_users",
        "archived_supported": True,
    },
}


# Archived streams full-sweep the v3 list endpoint with ``archived=true``;
# HubSpot doesn't accept an ``archivedAt`` query filter so the page list
# pulls everything every sync, but the stream filters records client-side
# by ``archivedAt > state`` so only newly archived records are emitted to
# the destination after the first sync.
_ARCHIVED_STREAM_SUFFIX = "_archived"


def _derive_archived(name: str, entry: Mapping) -> Mapping:
    return {
        "object_type": entry["object_type"],
        # Archived list endpoint has no server-side `archivedAt` filter; the
        # stream pages the whole archived set every sync and filters records
        # client-side via ``record.archivedAt > self._state``. State advances
        # to ``max(archivedAt)`` seen — first sync emits all archives, later
        # syncs emit only newly-archived rows.
        "cursor_field": "archivedAt",
        "search_cursor_property": None,
        "associations": list(entry.get("associations") or []),
        "silver_tag": entry.get("silver_tag"),
        "archived_supported": True,
        "is_archived": True,
        "live_stream_name": name,
    }


STREAM_REGISTRY: Mapping[str, Mapping] = {
    **_LIVE_STREAM_REGISTRY,
    **{
        f"{name}{_ARCHIVED_STREAM_SUFFIX}": _derive_archived(name, entry)
        for name, entry in _LIVE_STREAM_REGISTRY.items()
        if entry.get("archived_supported")
    },
}

# Curated default list — live streams only; archived siblings are appended at
# runtime by ``source._resolve_stream_list``.
CURATED_STREAMS = list(_LIVE_STREAM_REGISTRY.keys())

# Stream-name suffix used by the source to derive archived siblings.
ARCHIVED_STREAM_SUFFIX = _ARCHIVED_STREAM_SUFFIX

# Property scope: standard (``hubspotDefined``) properties are filtered
# through ``ALLOWED_PROPERTIES_BY_OBJECT`` so Bronze width stays bounded.
# Tenant-defined (``hubspotDefined=False``) properties always pass through
# and ride in ``custom_fields`` JSON.
ALLOWED_PROPERTIES_BY_OBJECT: Mapping[str, FrozenSet[str]] = {
    "contacts": frozenset({
        "email", "firstname", "lastname", "hubspot_owner_id", "lifecyclestage",
        "city", "state", "country", "jobtitle", "phone",
        "hs_lead_status", "hs_analytics_source",
    }),
    "companies": frozenset({
        "annualrevenue", "city", "country", "domain",
        "hubspot_owner_id", "industry", "name",
        "numberofemployees", "state",
        "lifecyclestage", "phone", "type",
    }),
    "deals": frozenset({
        "amount", "closedate", "dealname", "dealstage", "dealtype",
        "hs_analytics_source", "hs_deal_stage_probability",
        "hs_manual_forecast_category", "hs_is_closed", "hs_is_closed_won",
        "hubspot_owner_id", "pipeline",
        "closed_lost_reason", "hs_priority", "description",
    }),
    "calls": frozenset({
        "hs_call_direction", "hs_call_disposition", "hs_call_duration",
        "hs_call_title", "hs_timestamp", "hubspot_owner_id",
        "hs_call_status",
    }),
    "emails": frozenset({
        "hs_email_direction", "hs_email_status", "hs_email_subject",
        "hs_timestamp", "hubspot_owner_id",
    }),
    "meetings": frozenset({
        "hs_meeting_end_time", "hs_meeting_location", "hs_meeting_outcome",
        "hs_meeting_start_time", "hs_meeting_title",
        "hs_timestamp", "hubspot_owner_id",
        "hs_meeting_external_url", "hs_internal_meeting_notes",
    }),
    "tasks": frozenset({
        "hs_task_priority", "hs_task_status", "hs_task_subject",
        "hs_task_type", "hs_timestamp", "hubspot_owner_id",
        "hs_task_completion_date",
    }),
    "owners": frozenset(),
    "leads": frozenset(),
    "tickets": frozenset(),
}
