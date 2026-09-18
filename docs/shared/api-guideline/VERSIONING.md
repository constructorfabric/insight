# API Versioning & Compatibility

> Source: [constructorfabric/DNA — REST/VERSIONING.md](https://github.com/constructorfabric/DNA/blob/main/REST/VERSIONING.md)

This document defines the versioning strategy, compatibility rules, and deprecation practices for all Insight REST APIs.

---

## Table of Contents

- [Versioning Strategy](#versioning-strategy)
- [Compatibility Rules](#compatibility-rules)
- [Breaking vs Non-Breaking Changes](#breaking-vs-non-breaking-changes)
- [Deprecation Process](#deprecation-process)
- [Version Migration](#version-migration)
- [Client Guidelines](#client-guidelines)
- [Best Practices](#best-practices)

---

## Versioning Strategy

### Version Format

- **Path-based versioning**: `/v1`, `/v2`, etc.
- **Semantic structure**: Major version only in URL path
- **Internal versioning**: Use semantic versioning (e.g., `1.2.3`) internally for tracking

```
https://api.example.com/v1/users
https://api.example.com/v2/users
```

### Version Lifecycle

| Stage | Example | Description |
|-------|---------|-------------|
| Development | `v1-alpha`, `v1-beta` | Internal/staging only |
| Stable | `v1` | Production ready |
| Deprecated | `v1` (with `Deprecation` header) | Sunset period active |
| Retired | — | No longer available; returns `410 Gone` |

---

## Compatibility Rules

### Backward Compatibility (Within Major Version)

**MUST maintain compatibility** for:
- Existing endpoint URLs
- Request/response field names and types
- HTTP status codes for existing scenarios
- Authentication mechanisms
- Core functionality behavior

**MAY be added** without breaking compatibility:
- New optional fields in requests
- New fields in responses
- New endpoints
- New HTTP methods on existing resources
- Additional enum values (with graceful degradation)
- Additional query parameters

### Forward Compatibility (Client Resilience)

Clients **MUST** be designed to:
- Ignore unknown fields in responses
- Handle additional enum values gracefully
- Not rely on field order in JSON objects
- Treat missing optional fields as absent/default

---

## Breaking vs Non-Breaking Changes

### ✅ Non-Breaking Changes

**Request Changes**:
- Adding optional fields
- Adding optional query parameters
- Adding new enum values to optional fields
- Making required fields optional
- Relaxing validation rules

**Response Changes**:
- Adding new fields
- Adding new enum values
- Adding new optional headers
- Providing more detailed error messages
- Improving performance/response times

**Endpoint Changes**:
- Adding new endpoints
- Adding new HTTP methods to existing resources
- Adding new optional headers

### ❌ Breaking Changes

**Request Changes**:
- Removing fields
- Making optional fields required
- Changing field types
- Changing field semantics
- Removing enum values
- Tightening validation rules
- Changing URL structure

**Response Changes**:
- Removing fields
- Changing field types
- Changing field semantics
- Removing enum values
- Changing HTTP status codes for existing scenarios
- Changing error response format

**Endpoint Changes**:
- Removing endpoints
- Removing HTTP methods
- Changing authentication requirements
- Changing rate limits significantly

---

## Deprecation Process

### Deprecation Headers

When deprecating an API endpoint or version, include these headers on every response:

```http
Deprecation: true
Sunset: Sat, 31 Dec 2025 23:59:59 GMT
Link: <https://docs.api.example.com/migration/v1-to-v2>; rel="deprecation"
```

The response body remains unchanged — no wrapper, no additional fields. Normal format applies (`items` + `page_info` for lists, direct fields for single objects).

### Deprecation Timeline

| Phase | Timing | Actions |
|-------|--------|---------|
| Announcement | T-12 months | Publish deprecation notice, update docs, add deprecation headers |
| Warning Period | T-6 months | Log deprecation warnings, notify active API consumers, provide migration guides |
| Sunset Period | T-3 months | Increase warning frequency, consider rate-limiting deprecated endpoints, direct support for migration |
| Retirement | T-0 | Remove deprecated version, return `410 Gone` for deprecated endpoints |

---

## Version Migration

### Migration Strategy

**Gradual Migration**:
- Support overlapping versions (typically 2 major versions simultaneously)
- Provide clear migration paths
- Offer dual-write capabilities for data changes
- Maintain feature parity during transition

**Migration Tools**:
- Automated compatibility checkers
- Code generation for new SDKs
- Migration scripts for common patterns
- Sandbox environments for testing

### OpenAPI Versioning

```yaml
openapi: 3.1.0
info:
  title: Example API
  version: 2.1.0
  description: |
    API version 2.1.0

    **Deprecation Notice**: This API version will be sunset on 2025-12-31.
    Please migrate to v3. See [migration guide](https://docs.api.example.com/migration/v2-to-v3).

servers:
  - url: https://api.example.com/v2
    description: Production v2
  - url: https://api.example.com/v1
    description: Production v1 (deprecated)
    x-deprecated: true
    x-sunset: "2025-12-31T23:59:59.000Z"
```

---

## Client Guidelines

### Robust Client Design

**Version Handling**:
- Always specify API version explicitly in the URL path
- Handle version-specific responses gracefully
- Implement fallback mechanisms for deprecated features

**Error Handling** for unsupported versions:

```json
{
  "type": "https://api.example.com/errors/version-not-supported",
  "title": "API Version Not Supported",
  "status": 400,
  "detail": "API version 'v3' is not supported. Supported versions: v1, v2",
  "supported_versions": ["v1", "v2"],
  "latest_version": "v2"
}
```

**Future-Proofing**:
- Use strongly-typed models with unknown field handling
- Implement graceful degradation for new enum values
- Version your own client SDKs alongside API versions

### SDK Versioning

```
SDK v2.1.0 → API v2
SDK v2.2.0 → API v2 (new features)
SDK v3.0.0 → API v3
```

- Major SDK version tracks API major version
- Minor SDK updates for new features within API version
- Patch SDK updates for bug fixes

---

## Best Practices

### Do's

- ✅ Plan major versions carefully — they're expensive
- ✅ Communicate changes early and often
- ✅ Provide comprehensive migration documentation
- ✅ Support at least 2 major versions simultaneously
- ✅ Use feature flags for gradual rollouts
- ✅ Monitor usage of deprecated endpoints
- ✅ Test backward compatibility automatically

### Don'ts

- ❌ Don't break backward compatibility within major versions
- ❌ Don't remove versions without sufficient notice
- ❌ Don't introduce breaking changes as minor updates
- ❌ Don't rely on clients to handle breaking changes gracefully
- ❌ Don't version internal implementation details
- ❌ Don't create versions for every small change

### Version Planning Checklist

Before creating a new major version:

- [ ] Document all breaking changes
- [ ] Provide migration guide with examples
- [ ] Update OpenAPI specifications
- [ ] Generate new client SDKs
- [ ] Plan deprecation timeline for previous version
- [ ] Set up monitoring for version adoption
- [ ] Test migration scripts with real data
- [ ] Communicate timeline to all stakeholders

---

## References

- [Semantic Versioning](https://semver.org/)
- [RFC 8594 — The Sunset HTTP Header Field](https://tools.ietf.org/html/rfc8594)
- [API Deprecation Guidelines](https://tools.ietf.org/html/draft-ietf-httpapi-deprecation-header)
