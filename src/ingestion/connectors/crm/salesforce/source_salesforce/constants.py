"""Salesforce sobject and field-type constants.

The blocklists (``UNSUPPORTED_BULK_API_SALESFORCE_OBJECTS`` etc.) reflect
platform-wide API limitations maintained by Salesforce; our curated
``CRM_STREAMS`` sits on top of them.
"""

# ------- SF API version -------------------------------------------------------

API_VERSION = "v62.0"

# ------- Field-type buckets used in describe -> JSON-schema mapping -----------

STRING_TYPES = [
    "byte",
    "combobox",
    "complexvalue",
    "datacategorygroupreference",
    "email",
    "encryptedstring",
    "id",
    "json",
    "masterrecord",
    "multipicklist",
    "phone",
    "picklist",
    "reference",
    "string",
    "textarea",
    "time",
    "url",
]
NUMBER_TYPES = ["currency", "double", "long", "percent"]
DATE_TYPES = ["date", "datetime"]
LOOSE_TYPES = [
    "anyType",
    # A calculated field's type can be any formula data type. Docs:
    # https://developer.salesforce.com/docs/atlas.en-us.api.meta/api/field_types.htm
    "calculated",
]

# ------- Sobject blocklists ---------------------------------------------------

QUERY_RESTRICTED_SALESFORCE_OBJECTS = [
    "Announcement",
    "AppTabMember",
    "CollaborationGroupRecord",
    "ColorDefinition",
    "ContentFolderItem",
    "ContentFolderMember",
    "DataStatistics",
    "DatacloudDandBCompany",
    "EntityParticle",
    "FieldDefinition",
    "FieldHistoryArchive",
    "FlexQueueItem",
    "FlowVariableView",
    "FlowVersionView",
    "IconDefinition",
    "IdeaComment",
    "NetworkUserHistoryRecent",
    "OwnerChangeOptionInfo",
    "PicklistValueInfo",
    "PlatformAction",
    "RelationshipDomain",
    "RelationshipInfo",
    "SearchLayout",
    "SiteDetail",
    "UserEntityAccess",
    "UserFieldAccess",
    "Vote",
]

QUERY_INCOMPATIBLE_SALESFORCE_OBJECTS = [
    "AIPredictionEvent",
    "ActivityHistory",
    "AggregateResult",
    "ApiAnomalyEvent",
    "ApiEventStream",
    "AssetTokenEvent",
    "AsyncOperationEvent",
    "AsyncOperationStatus",
    "AttachedContentDocument",
    "AttachedContentNote",
    "BatchApexErrorEvent",
    "BulkApiResultEvent",
    "CombinedAttachment",
    "ConcurLongRunApexErrEvent",
    "ContentBody",
    "CredentialStuffingEvent",
    "DataType",
    "DatacloudAddress",
    "EmailStatus",
    "FeedLike",
    "FeedSignal",
    "FeedTrackedChange",
    "FlowExecutionErrorEvent",
    "FolderedContentDocument",
    "LightningUriEventStream",
    "ListViewChartInstance",
    "ListViewEventStream",
    "LoginAsEventStream",
    "LoginEventStream",
    "LogoutEventStream",
    "LookedUpFromActivity",
    "Name",
    "NoteAndAttachment",
    "OpenActivity",
    "OrgLifecycleNotification",
    "OutgoingEmail",
    "OutgoingEmailRelation",
    "OwnedContentDocument",
    "PlatformStatusAlertEvent",
    "ProcessExceptionEvent",
    "ProcessInstanceHistory",
    "QuoteTemplateRichTextData",
    "RemoteKeyCalloutEvent",
    "ReportAnomalyEvent",
    "ReportEventStream",
    "SessionHijackingEvent",
    "UriEventStream",
    "UserRecordAccess",
]

UNSUPPORTED_BULK_API_SALESFORCE_OBJECTS = [
    "AcceptedEventRelation",
    "AssetTokenEvent",
    "Attachment",
    "AttachedContentNote",
    "CaseStatus",
    "ContractStatus",
    "DeclinedEventRelation",
    "EventWhoRelation",
    "FieldSecurityClassification",
    "KnowledgeArticle",
    "KnowledgeArticleVersion",
    "KnowledgeArticleVersionHistory",
    "KnowledgeArticleViewStat",
    "KnowledgeArticleVoteStat",
    "OrderStatus",
    "PartnerRole",
    "QuoteTemplateRichTextData",
    "RecentlyViewed",
    "ServiceAppointmentStatus",
    "ShiftStatus",
    "SolutionStatus",
    "TaskPriority",
    "TaskStatus",
    "TaskWhoRelation",
    "UndecidedEventRelation",
    "WorkOrderLineItemStatus",
    "WorkOrderStatus",
    "UserRecordAccess",
    "OwnedContentDocument",
    "OpenActivity",
    "NoteAndAttachment",
    "Name",
    "LookedUpFromActivity",
    "FolderedContentDocument",
    "ContentFolderItem",
    "CombinedAttachment",
    "CaseTeamTemplateRecord",
    "CaseTeamTemplateMember",
    "CaseTeamTemplate",
    "CaseTeamRole",
    "CaseTeamMember",
    "AttachedContentDocument",
    "AggregateResult",
    "ChannelProgramLevelShare",
    "AccountBrandShare",
    "AccountFeed",
    "AssetFeed",
]

UNSUPPORTED_FILTERING_STREAMS = [
    "ActivityFieldHistory",
    "ApiEvent",
    "BulkApiResultEventStore",
    "ContentDocumentLink",
    "EmbeddedServiceDetail",
    "EmbeddedServiceLabel",
    "FormulaFunction",
    "FormulaFunctionAllowedType",
    "FormulaFunctionCategory",
    "IdentityProviderEventStore",
    "IdentityVerificationEvent",
    "LightningUriEvent",
    "ListViewEvent",
    "LoginAsEvent",
    "LoginEvent",
    "LogoutEvent",
    "Publisher",
    "RecordActionHistory",
    "ReportEvent",
    "TabDefinition",
    "UriEvent",
]

UNSUPPORTED_STREAMS = ["ActivityMetric", "ActivityMetricRollup"]

PARENT_SALESFORCE_OBJECTS = {
    "ContentDocumentLink": {
        "parent_name": "ContentDocument",
        "field": "Id",
        "schema_minimal": {
            "properties": {
                "Id": {"type": ["string", "null"]},
                "SystemModstamp": {"type": ["string", "null"], "format": "date-time"},
            }
        },
    }
}

# ------- Token / request limits -----------------------------------------------

# Refresh Salesforce access token every 30 minutes, well before the default
# 2-hour session timeout. Prevents INVALID_SESSION_ID mid-sync on large Bulk jobs.
TOKEN_REFRESH_INTERVAL_SECONDS = 1800

# SF request size limit (SOQL query length).
# https://developer.salesforce.com/docs/atlas.en-us.salesforce_app_limits_cheatsheet.meta/salesforce_app_limits_cheatsheet/salesforce_app_limits_platform_api.htm
REQUEST_SIZE_LIMITS = 16_384

# Connection pool size for parallel HTTP tasks.
PARALLEL_TASKS_SIZE = 100

# ------- Curated stream list (Insight-specific) -------------------------------

# Streams collected by default for CRM analytics + HubSpot-parity.
# Operator can override via config.salesforce_streams to add/remove.
#
# Cut to a 10-stream "max value per stream" set. Entries commented out below
# remain supported by the connector — re-enable by moving them into the active
# list (or per-tenant via ``salesforce_streams`` config).
CRM_STREAMS = [
    # Core deal lifecycle
    "Account",
    "Contact",
    "Opportunity",
    "OpportunityHistory",      # stage/amount change history — pipeline velocity
    "Task",
    "Event",
    "User",
    # Funnel + extensions
    "Lead",
    "Case",                    # support tickets — CX analytics
    "OpportunityContactRole",  # buyer-committee / multi-threading
    # --- Disabled for now. Re-enable when the use case lands. ---
    # "OpportunityLineItem",   # revenue-by-product (needs Product2 + Pricebook* joins)
    # "Product2",              # product catalog reference
    # "Pricebook2",            # pricebook reference
    # "PricebookEntry",        # pricebook SKU reference
    # "CampaignMember",        # marketing attribution enrollments
    # "Campaign",              # reference for CampaignMember
]
