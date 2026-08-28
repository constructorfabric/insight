"""Copy the data mover's account of every sync into the connector sync ledger.

The mover is the only record of what synced, its history is bounded, and it
disappears with a deleted connection. This package runs inside the reconcile
tick, which already authenticates to the mover, and copies that account into
`ingestion_history.sync_events` so the connector health page can answer from
one relation while everything above it is down.

Spec: docs/components/backend/analytics/specs/connector-health.
"""
