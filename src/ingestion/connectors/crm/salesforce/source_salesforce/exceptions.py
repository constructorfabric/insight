"""Exception hierarchy for the Salesforce connector."""


class SalesforceException(Exception):
    """Base class for Salesforce-specific errors."""


class TypeSalesforceException(SalesforceException):
    """Unknown Salesforce field type encountered during schema generation."""
