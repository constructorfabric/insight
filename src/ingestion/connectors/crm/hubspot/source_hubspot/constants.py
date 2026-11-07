"""HubSpot stream registry, Bronze column allowlist, and API limits."""

from collections.abc import Mapping

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
        "kind": "crm_object",
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
        "kind": "crm_object",
        "object_type": "companies",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": [],
        "silver_tag": "silver:class_crm_accounts",
        "archived_supported": True,
    },
    "deals": {
        "kind": "crm_object",
        "object_type": "deals",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["companies", "contacts"],
        "silver_tag": "silver:class_crm_deals",
        "archived_supported": True,
    },
    "engagements_calls": {
        "kind": "crm_object",
        "object_type": "calls",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        "archived_supported": True,
    },
    "engagements_emails": {
        "kind": "crm_object",
        "object_type": "emails",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        "archived_supported": True,
    },
    "engagements_meetings": {
        "kind": "crm_object",
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
        "kind": "crm_object",
        "object_type": "tasks",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        "archived_supported": True,
    },
    "engagements_notes": {
        "kind": "crm_object",
        "object_type": "notes",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        "archived_supported": True,
    },
    "engagements_communications": {
        "kind": "crm_object",
        "object_type": "communications",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_activities",
        # HTTP 400 "Paging through deleted objects is not yet supported for
        # object type 0-18 (COMMUNICATION)".
        "archived_supported": False,
    },
    "leads": {
        "kind": "crm_object",
        "object_type": "leads",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies"],
        "silver_tag": "silver:class_crm_leads",
        "archived_supported": True,
    },
    "tickets": {
        "kind": "crm_object",
        "object_type": "tickets",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": ["contacts", "companies", "deals"],
        "silver_tag": "silver:class_crm_tickets",
        "archived_supported": True,
    },
    # owners is NOT a CRM object — different endpoint shape (/crm/v3/owners).
    # Handled by a dedicated stream class; no search endpoint, no custom
    # properties. The owners list endpoint doesn't accept an updatedAt
    # filter, so the stream pages the full owner set every sync but filters
    # records client-side by ``updatedAt > state``; only changed owners are
    # emitted to the destination after the first sync.
    "owners": {
        "kind": "owners",
        "object_type": "owners",
        "cursor_field": "updatedAt",
        "search_cursor_property": None,
        "associations": [],
        "silver_tag": "silver:class_crm_users",
        "archived_supported": True,
    },
}

# Streams that are not CRM object listings: pipeline definitions, property
# definitions and property-change history. None has an archived sibling.
_AUX_STREAM_REGISTRY: Mapping[str, Mapping] = {
    "deal_pipelines": {
        "kind": "pipelines",
        "object_type": "deals",
        "cursor_field": "updatedAt",
        "search_cursor_property": None,
        "associations": [],
        "silver_tag": "silver:class_crm_pipelines",
        "archived_supported": False,
    },
    "ticket_pipelines": {
        "kind": "pipelines",
        "object_type": "tickets",
        "cursor_field": "updatedAt",
        "search_cursor_property": None,
        "associations": [],
        "silver_tag": "silver:class_crm_pipelines",
        "archived_supported": False,
    },
    "property_definitions": {
        "kind": "property_definitions",
        "object_type": None,
        "cursor_field": "updatedAt",
        "search_cursor_property": None,
        "associations": [],
        "silver_tag": "silver:class_crm_property_definitions",
        "archived_supported": False,
    },
    "deals_property_history": {
        "kind": "property_history",
        "object_type": "deals",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": [],
        "silver_tag": "silver:class_crm_field_history",
        "archived_supported": False,
        "history_properties": (
            "dealstage",
            "pipeline",
            "amount",
            "amount_in_home_currency",
            "deal_currency_code",
            "closedate",
            "hubspot_owner_id",
            "hs_manual_forecast_category",
        ),
    },
    "tickets_property_history": {
        "kind": "property_history",
        "object_type": "tickets",
        "cursor_field": "updatedAt",
        "search_cursor_property": "hs_lastmodifieddate",
        "associations": [],
        "silver_tag": "silver:class_crm_field_history",
        "archived_supported": False,
        "history_properties": ("hs_pipeline_stage", "hs_pipeline", "hubspot_owner_id", "hs_ticket_priority"),
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
        "kind": entry["kind"],
        "object_type": entry["object_type"],
        # Archived list endpoint has no server-side `archivedAt` filter; the
        # stream pages the whole archived set every sync and filters records
        # client-side via ``record.archivedAt > self._state``. State advances
        # to ``max(archivedAt)`` seen — first sync emits all archives, later
        # syncs emit only newly-archived rows.
        # HubSpot omits ``archivedAt`` on archived owners; their archive
        # stamps ``updatedAt`` instead.
        "cursor_field": "updatedAt" if entry["kind"] == "owners" else "archivedAt",
        "search_cursor_property": None,
        "associations": list(entry.get("associations") or []),
        "silver_tag": entry.get("silver_tag"),
        "archived_supported": True,
        "is_archived": True,
        "live_stream_name": name,
    }


STREAM_REGISTRY: Mapping[str, Mapping] = {
    **_LIVE_STREAM_REGISTRY,
    **_AUX_STREAM_REGISTRY,
    **{
        f"{name}{_ARCHIVED_STREAM_SUFFIX}": _derive_archived(name, entry)
        for name, entry in _LIVE_STREAM_REGISTRY.items()
        if entry.get("archived_supported")
    },
}

# Curated default list — live and auxiliary streams; archived siblings are
# appended at runtime by ``source._resolve_stream_list``.
CURATED_STREAMS = [*_LIVE_STREAM_REGISTRY.keys(), *_AUX_STREAM_REGISTRY.keys()]

# Object types whose property definitions the ``property_definitions`` stream
# lists — every CRM object a live stream syncs.
PROPERTY_DEFINITION_OBJECT_TYPES = tuple(
    entry["object_type"] for entry in _LIVE_STREAM_REGISTRY.values() if entry["kind"] == "crm_object"
)

# HubSpot caps a batch_read that requests ``propertiesWithHistory`` at 50 inputs.
HISTORY_BATCH_READ_LIMIT = 50

# Stream-name suffix used by the source to derive archived siblings.
ARCHIVED_STREAM_SUFFIX = _ARCHIVED_STREAM_SUFFIX

# Bronze column scope: these properties get a dedicated
# ``properties_{name}`` column, and they alone define the advertised schema —
# it is identical for every portal, and a property the portal doesn't define
# stays NULL. Every other property the portal returns still ships, inside
# ``raw_data``.
ALLOWED_PROPERTIES_BY_OBJECT: Mapping[str, frozenset[str]] = {
    "contacts": frozenset(
        {
            "email",
            "firstname",
            "lastname",
            "hubspot_owner_id",
            "lifecyclestage",
            "city",
            "state",
            "country",
            "jobtitle",
            "phone",
            "hs_lead_status",
            "hs_analytics_source",
        }
    ),
    "companies": frozenset(
        {
            "annualrevenue",
            "city",
            "country",
            "domain",
            "hubspot_owner_id",
            "industry",
            "name",
            "numberofemployees",
            "state",
            "lifecyclestage",
            "phone",
            "type",
        }
    ),
    "deals": frozenset(
        {
            "amount",
            "amount_in_home_currency",
            "closedate",
            "dealname",
            "dealstage",
            "dealtype",
            "hs_analytics_source",
            "hs_deal_stage_probability",
            "hs_manual_forecast_category",
            "hs_is_closed",
            "hs_is_closed_won",
            "hs_acv",
            "hs_tcv",
            "hs_arr",
            "hubspot_owner_id",
            "hs_created_by_user_id",
            "pipeline",
            "closed_lost_reason",
            "hs_priority",
            "description",
            "deal_currency_code",
            "hs_exchange_rate",
        }
    ),
    "calls": frozenset(
        {
            "hs_call_direction",
            "hs_call_disposition",
            "hs_call_duration",
            "hs_call_title",
            "hs_timestamp",
            "hubspot_owner_id",
            "hs_created_by_user_id",
            "hs_call_status",
            "hs_call_body",
        }
    ),
    "emails": frozenset(
        {
            "hs_email_direction",
            "hs_email_status",
            "hs_email_subject",
            "hs_timestamp",
            "hubspot_owner_id",
            "hs_created_by_user_id",
            "hs_email_text",
        }
    ),
    "meetings": frozenset(
        {
            "hs_meeting_end_time",
            "hs_meeting_location",
            "hs_meeting_outcome",
            "hs_meeting_start_time",
            "hs_meeting_title",
            "hs_timestamp",
            "hubspot_owner_id",
            "hs_created_by_user_id",
            "hs_meeting_external_url",
            "hs_internal_meeting_notes",
            "hs_meeting_body",
        }
    ),
    "tasks": frozenset(
        {
            "hs_task_priority",
            "hs_task_status",
            "hs_task_subject",
            "hs_task_type",
            "hs_timestamp",
            "hubspot_owner_id",
            "hs_created_by_user_id",
            "hs_task_completion_date",
            "hs_task_body",
        }
    ),
    "notes": frozenset(
        {"hs_note_body", "hs_timestamp", "hubspot_owner_id", "hs_created_by_user_id", "hs_attachment_ids"}
    ),
    "communications": frozenset(
        {
            "hs_communication_channel_type",
            "hs_communication_logged_from",
            "hs_communication_body",
            "hs_timestamp",
            "hubspot_owner_id",
            "hs_created_by_user_id",
        }
    ),
    "leads": frozenset(
        {
            "hs_lead_name",
            "hs_pipeline",
            "hs_pipeline_stage",
            "hubspot_owner_id",
            "hs_created_by_user_id",
            "hs_lead_type",
            "hs_lead_label",
            "hs_primary_contact_id",
            "hs_primary_company_id",
            "hs_lead_disqualification_reason",
        }
    ),
    "tickets": frozenset(
        {
            "subject",
            "content",
            "hs_pipeline",
            "hs_pipeline_stage",
            "hs_ticket_priority",
            "hs_ticket_category",
            "hubspot_owner_id",
            "hs_created_by_user_id",
            "hubspot_team_id",
            "closed_date",
            "hs_is_closed",
            "hs_resolution",
            "source_type",
        }
    ),
    "owners": frozenset(),
}

# Free-text bodies get a wider per-value cap than other ``properties_*``
# columns; ``raw_data`` keeps the default cap for every value.
BODY_PROPERTIES: frozenset[str] = frozenset(
    {
        "hs_call_body",
        "hs_email_text",
        "hs_meeting_body",
        "hs_internal_meeting_notes",
        "hs_task_body",
        "hs_note_body",
        "hs_communication_body",
        "content",
        "description",
    }
)
