#!/usr/bin/env bash
# Insight platform — seed one dev impersonation person.
#
# The compose stack has no ingestion pipeline (Airbyte + dbt + the
# /v1/persons-seed flow live in the k8s path). For local FE development
# the frontend needs at least one person whose value_id matches
# VITE_DEV_USER_EMAIL. This script inserts that row directly into
# identity's MariaDB.
#
# Idempotent: re-running it does NOT create duplicates because the
# table's unique key (tenant, person, source_type, source_id, value_type,
# value_hash) absorbs the second INSERT IGNORE.
#
# Run AFTER ./dev-compose-up.sh (identity must have applied its
# migrations, creating the persons table).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

ENV_FILE="${1:-.env.compose}"
[[ -f "$ENV_FILE" ]] || { echo "ERROR: env file not found: $ENV_FILE" >&2; exit 1; }
set -a; source "$ENV_FILE"; set +a

EMAIL="${VITE_DEV_USER_EMAIL:-}"
if [[ -z "$EMAIL" ]]; then
  echo "ERROR: VITE_DEV_USER_EMAIL is empty in $ENV_FILE." >&2
  echo "       Set it to an email you want to impersonate (e.g. you@yourorg.com)" >&2
  echo "       and re-run." >&2
  exit 1
fi

TENANT="${TENANT_DEFAULT_ID:-00000000-df51-5b42-9538-d2b56b7ee953}"
PERSON_ID="${VITE_DEV_PERSON_ID:-00000000-0000-0000-0000-000000000010}"
SOURCE_TYPE="dev-seed"

echo "Seeding dev impersonation:"
echo "  email     = $EMAIL"
echo "  tenant_id = $TENANT"
echo "  person_id = $PERSON_ID"

# Wait briefly for the persons table to exist — identity may still be
# applying migrations on a fresh boot.
for attempt in 1 2 3 4 5 6 7 8 9 10; do
  if docker exec insight-mariadb mariadb \
       -u"${MARIADB_USER:-insight}" -p"${MARIADB_PASSWORD:-insight-local}" identity \
       -e 'SELECT 1 FROM persons LIMIT 1' >/dev/null 2>&1; then
    break
  fi
  echo "  waiting for persons table (attempt $attempt)..."
  sleep 2
done

docker exec -i insight-mariadb mariadb \
  -u"${MARIADB_USER:-insight}" -p"${MARIADB_PASSWORD:-insight-local}" identity <<SQL
INSERT IGNORE INTO persons (
  value_type, insight_source_type, insight_source_id, insight_tenant_id,
  value_id, person_id, author_person_id, reason
) VALUES (
  'email',
  '${SOURCE_TYPE}',
  UNHEX('00000000000000000000000000000001'),
  UNHEX(REPLACE('${TENANT}', '-', '')),
  '${EMAIL}',
  UNHEX(REPLACE('${PERSON_ID}', '-', '')),
  UNHEX('00000000000000000000000000000000'),
  'dev-compose-seed.sh'
);
SELECT
  LOWER(HEX(insight_tenant_id)) AS tenant,
  value_id                       AS email,
  LOWER(HEX(person_id))          AS person_id
FROM persons
WHERE value_id = '${EMAIL}';
SQL

echo "Done. The frontend should now bind VITE_DEV_USER_EMAIL=${EMAIL} successfully."
