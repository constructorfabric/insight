# Reconcile against a real Airbyte

`stand.sh` raises a dedicated kind cluster with Airbyte, Argo Workflows and a
single-node ClickHouse, versions pinned with `deploy/gitops`. The tests in this
directory run `main.sh reconcile` against it and read the result back from
Airbyte and Kubernetes. Syncs are never triggered, so no connector data is read.

Requires docker, kind, helm, kubectl and bash 4+ (`brew install bash` on macOS).

```bash
src/ingestion/reconcile-connectors/tests/live/stand.sh up     # ~15 min the first time
eval "$(src/ingestion/reconcile-connectors/tests/live/stand.sh env)"
uvx --with pyyaml pytest src/ingestion/reconcile-connectors/tests/live
src/ingestion/reconcile-connectors/tests/live/stand.sh down
```

`stand.sh env` exports a `KUBECONFIG` that names only the stand's cluster. The
tests skip unless `KUBECONFIG` lies in the stand's state directory and holds only
the `kind-insight-reconcile` context — a guard against an accidental run, not a
check of which cluster answers. Reconcile calls kubectl without `--context` and
these tests delete Secrets, sources and schedules. Without the stand's
environment the tests skip, so the CI run of `tests/` is unaffected.

The ledger sweep at the end of a reconcile tick reads ClickHouse at the
in-cluster address, which the host cannot resolve; its warning in the log is
expected here.
