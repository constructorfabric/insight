#!/usr/bin/env bash
# Manual smoke for the OrgChart Visibility CRUD endpoints introduced in #346 steps 4-5.
#
# Exercises the happy path on /v1/visibility, /v1/roles, /v1/person-roles
# (admin-only, see ADR-0012) plus the two error guards:
#   - 422 urn:insight:error:role_in_use       (ADR-0013)
#   - 422 urn:insight:error:last_admin_protected (ADR-0014)
# Also covers the 401/403 admin gate.
#
# Assumes the service is reachable at $BASE_URL and the bootstrap admin
# has already been minted (the identity-resolution migrate step seeds it
# when APP__gears__identity_resolution__config__bootstrap_admin_person_id
# is set).
#
# Usage:
#   BASE_URL=http://localhost:8082 \
#   TENANT_ID=<uuid> \
#   ADMIN_PERSON_ID=<uuid present in person_roles with role_id=admin> \
#   ./src/backend/services/identity-resolution/seed/smoke-orgchart-visibility.sh
#
# Optional overrides:
#   VIEWER_PERSON_ID, VIEWED_PERSON_ID — must exist in persons for this tenant.
#   NON_ADMIN_PERSON_ID — a person without an admin assignment; used to
#                        prove the 403 branch of CallerAdminCheck.
set -euo pipefail

: "${BASE_URL:?BASE_URL required, e.g. http://localhost:8082}"
: "${TENANT_ID:?TENANT_ID required}"
: "${ADMIN_PERSON_ID:?ADMIN_PERSON_ID required}"
VIEWER_PERSON_ID="${VIEWER_PERSON_ID:-$(uuidgen)}"
VIEWED_PERSON_ID="${VIEWED_PERSON_ID:-$(uuidgen)}"
NON_ADMIN_PERSON_ID="${NON_ADMIN_PERSON_ID:-$(uuidgen)}"

H_TENANT=(-H "X-Insight-Tenant-Id: ${TENANT_ID}")
H_ADMIN=(-H "X-Insight-Person-Id: ${ADMIN_PERSON_ID}")
H_NONADMIN=(-H "X-Insight-Person-Id: ${NON_ADMIN_PERSON_ID}")
H_JSON=(-H "Content-Type: application/json")

ROLE_ADMIN_ID="a4d11000-0000-4000-8000-000000000001"

step() { printf "\n=== %s ===\n" "$1"; }

# ---------------------------------------------------------------- admin gate

step "401 — no caller header"
curl -sS -o /dev/null -w "  HTTP %{http_code}\n" \
  "${H_TENANT[@]}" "${BASE_URL}/v1/visibility"

step "403 — non-admin caller"
curl -sS -o /dev/null -w "  HTTP %{http_code}\n" \
  "${H_TENANT[@]}" "${H_NONADMIN[@]}" "${BASE_URL}/v1/visibility"

# ---------------------------------------------------------------- visibility

step "POST /v1/visibility — mint a grant"
VIS_RESP=$(curl -sS -X POST "${BASE_URL}/v1/visibility" \
  "${H_TENANT[@]}" "${H_ADMIN[@]}" "${H_JSON[@]}" \
  -d "{\"viewer_person_id\":\"${VIEWER_PERSON_ID}\",\"viewed_person_id\":\"${VIEWED_PERSON_ID}\",\"reason\":\"smoke\"}")
echo "${VIS_RESP}"
VIS_ID=$(echo "${VIS_RESP}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["visibility_id"])')

step "GET /v1/visibility — list (active only)"
curl -sS "${BASE_URL}/v1/visibility?active=true&limit=5" "${H_TENANT[@]}" "${H_ADMIN[@]}"
echo

step "DELETE /v1/visibility/{id} — soft-delete the grant"
curl -sS -o /dev/null -w "  HTTP %{http_code}\n" -X DELETE \
  "${BASE_URL}/v1/visibility/${VIS_ID}" \
  "${H_TENANT[@]}" "${H_ADMIN[@]}" "${H_JSON[@]}" \
  -d '{"reason":"smoke revoke"}'

# ---------------------------------------------------------------- roles

step "POST /v1/roles — create a temporary role"
ROLE_RESP=$(curl -sS -X POST "${BASE_URL}/v1/roles" \
  "${H_TENANT[@]}" "${H_ADMIN[@]}" "${H_JSON[@]}" \
  -d '{"name":"smoke-temp-role"}')
echo "${ROLE_RESP}"
ROLE_ID=$(echo "${ROLE_RESP}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["role_id"])')

step "GET /v1/roles — list"
curl -sS "${BASE_URL}/v1/roles" "${H_TENANT[@]}" "${H_ADMIN[@]}"
echo

step "DELETE /v1/roles/{id} — hard delete (no assignments)"
curl -sS -o /dev/null -w "  HTTP %{http_code}\n" -X DELETE \
  "${BASE_URL}/v1/roles/${ROLE_ID}" "${H_TENANT[@]}" "${H_ADMIN[@]}"

# ---------------------------------------------------------------- role_in_use guard

step "422 urn:insight:error:role_in_use — admin role has bootstrap assignment"
curl -sS -X DELETE "${BASE_URL}/v1/roles/${ROLE_ADMIN_ID}" \
  "${H_TENANT[@]}" "${H_ADMIN[@]}"
echo

# ---------------------------------------------------------------- person_roles

step "POST /v1/person-roles — assign admin role to VIEWER (second admin)"
PR_RESP=$(curl -sS -X POST "${BASE_URL}/v1/person-roles" \
  "${H_TENANT[@]}" "${H_ADMIN[@]}" "${H_JSON[@]}" \
  -d "{\"person_id\":\"${VIEWER_PERSON_ID}\",\"role_id\":\"${ROLE_ADMIN_ID}\"}")
echo "${PR_RESP}"
PR_ID=$(echo "${PR_RESP}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["person_role_id"])')

step "GET /v1/person-roles — list active"
curl -sS "${BASE_URL}/v1/person-roles?active=true" "${H_TENANT[@]}" "${H_ADMIN[@]}"
echo

step "DELETE /v1/person-roles/{id} — revoke the second admin (two admins remain → 204)"
curl -sS -o /dev/null -w "  HTTP %{http_code}\n" -X DELETE \
  "${BASE_URL}/v1/person-roles/${PR_ID}" "${H_TENANT[@]}" "${H_ADMIN[@]}" "${H_JSON[@]}" \
  -d '{"reason":"smoke revoke"}'

# ---------------------------------------------------------------- last_admin_protected guard

step "422 urn:insight:error:last_admin_protected — try revoking the only remaining admin"
# Find the bootstrap admin's person_role_id.
LAST_ADMIN_PR_ID=$(curl -sS \
  "${BASE_URL}/v1/person-roles?active=true&role=${ROLE_ADMIN_ID}" \
  "${H_TENANT[@]}" "${H_ADMIN[@]}" \
  | python3 -c 'import json,sys;d=json.load(sys.stdin);print(d["items"][0]["person_role_id"])')

curl -sS -X DELETE "${BASE_URL}/v1/person-roles/${LAST_ADMIN_PR_ID}" \
  "${H_TENANT[@]}" "${H_ADMIN[@]}" "${H_JSON[@]}" \
  -d '{"reason":"smoke last-admin attempt"}'
echo

step "smoke done"
