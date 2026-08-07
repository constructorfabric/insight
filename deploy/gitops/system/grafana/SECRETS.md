# Grafana — Secrets

Grafana itself needs no credentials to install. This file covers the one
optional Secret it does read: the ClickHouse login behind the `ClickHouse`
datasource in `values.yaml`.

## `grafana-clickhouse` — the ClickHouse datasource login

Grafana charts warehouse volume straight out of `bronze_*`, which needs a
ClickHouse user. It must not be the admin (`clickhouse.auth.username`, which
holds `CREATE USER` / `DROP TABLE` / `INSERT`) — a dashboard has no business
carrying any of that. So `make system-grafana` provisions a dedicated
`grafana_ro`:

```
make system-grafana
  ├── apply environments/<env>/sealed-secrets/insight-infra/grafana*-sealedsecret.yaml
  ├── system/grafana/provision-clickhouse-user.sh   ← role + user in ClickHouse
  └── helm upgrade --install grafana
```

**Why here and not in the umbrella chart.** The umbrella already provisions
ClickHouse users at deploy time (the `presentation` user for analytics), and
putting `grafana_ro` there would have been less code. It would also have been
backwards: ingestion ships to every instance, Grafana is optional
(`inventory.system.grafana`). A cluster without Grafana should carry no
`grafana_ro`, and the thing that installs Grafana is the thing that should
create its user.

### Required keys

| Key | Used for |
|---|---|
| `username` | the ClickHouse user Grafana connects as — always `grafana_ro` |
| `password` | its password; the provisioning script sets the same value in ClickHouse |

### Everything is optional, and degrades rather than fails

- no `grafana-clickhouse` Secret → the script provisions the role and no
  user, Grafana installs, and only the ClickHouse datasource fails its health
  check. The Loki dashboards are unaffected.
- ClickHouse not in this namespace (external / managed) → the script says so
  and skips; provision `grafana_ro` out-of-band with `clickhouse-role.sql`
  plus a `CREATE USER`, then seal the Secret.
- admin without `access_management` → same, warns and skips.

So the order of operations does not matter: install Grafana first and seal the
password later, or the reverse. Re-running `make system-grafana` converges.

### Adding it to an environment

```bash
# 1. Generate. Alphanumeric on purpose: the script refuses a password with a
#    quote, backslash or semicolon, because the value rides the SQL body of
#    the CREATE USER statement.
GRAFANA_PW="$(openssl rand -base64 30 | tr -d /=+ | head -c 32)"

# 2. Put it in your secret store as insight-<env>-grafana-clickhouse, as
#    single-line JSON (Passbolt's password field is single-line):
jq -cn --arg p "$GRAFANA_PW" \
  '{apiVersion:"v1",kind:"Secret",
    metadata:{name:"grafana-clickhouse",namespace:"insight-infra"},
    type:"Opaque",
    stringData:{username:"grafana_ro",password:$p}}'

# 3. Seal and deploy:
make seal-secret ENV=<env> NAMESPACE=insight-infra NAME=grafana-clickhouse
make system-grafana ENV=<env>
```

### Rotation

Update the secret-store entry, re-seal, re-run `make system-grafana`. The
script's `ALTER USER` converges ClickHouse to the new password in the same
run that hands it to Grafana, so there is no window where the two disagree —
unlike a credential split across two deploy pipelines.

### Verify

```bash
GRAFANA_PW="$(kubectl -n insight-infra get secret grafana-clickhouse \
  -o jsonpath='{.data.password}' | base64 -d)"

kubectl -n insight-infra exec clickhouse-shard0-0 -- \
  clickhouse-client --user grafana_ro --password "$GRAFANA_PW" \
  -q "SELECT currentUser(), count() FROM system.tables"
# writes must be refused (expect ACCESS_DENIED):
kubectl -n insight-infra exec clickhouse-shard0-0 -- \
  clickhouse-client --user grafana_ro --password "$GRAFANA_PW" \
  -q "CREATE TABLE default.evil (a Int) ENGINE=Memory"
```

The grant matrix is pinned by `tests/test_clickhouse_role.py` beside this
file — opt-in, runs the real script against a throwaway ClickHouse:

```bash
docker run -d --rm --name ch -p 38211:8123 \
  -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight \
  -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  clickhouse/clickhouse-server:25.7.5
GRAFANA_ROLE_TEST_CH_URL=http://localhost:38211 \
GRAFANA_ROLE_TEST_CH_USER=insight \
GRAFANA_ROLE_TEST_CH_PASSWORD=insight \
  python -m pytest system/grafana/tests -q
```

### Why the role grants `SELECT ON *.*`

Unlike the umbrella's `presentation_ro`, which enumerates its contract
databases, `grafana_ro` takes a wildcard. Every connector onboarding creates a
new `bronze_<name>` database, and a fixed list would silently drop that
connector out of the dashboards until someone noticed the gap.
Read-only-by-construction still holds: `SELECT` and `SHOW` are the only
privileges, so the wildcard only discloses data the dashboards exist to show.
