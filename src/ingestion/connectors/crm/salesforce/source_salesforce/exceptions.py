"""Exception hierarchy for the Salesforce connector."""

import logging

logger = logging.getLogger("airbyte")


class SalesforceException(Exception):
    """Base class for Salesforce-specific errors."""


class TypeSalesforceException(SalesforceException):
    """Unknown Salesforce field type encountered during schema generation."""


class BulkNotSupportedException(SalesforceException):
    """Bulk API 2.0 cannot be used for the given sobject.

    The caller catches this during stream instantiation and falls back to the
    REST ``/queryAll`` path. Triggered by responses like ``Selecting compound
    data not supported in Bulk Query`` or ``INVALIDENTITY ... is not supported
    by the Bulk API``.
    """


class TmpFileIOError(SalesforceException):
    """Failure writing or reading a temporary file during Bulk ingest."""

    def __init__(self, msg: str, err: str | None = None):
        full = f"{msg}. Error: {err}" if err else msg
        super().__init__(full)
        logger.error(full)
