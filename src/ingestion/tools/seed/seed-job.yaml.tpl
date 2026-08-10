# One-shot Job that runs this seeder against a Kubernetes stand.
#
# Rendered by `seed-stand.sh` with envsubst and applied to the cluster; that
# script resolves every variable below from the stand itself and refuses to
# render when one is empty, so this file is also the reference manifest — the
# thing that runs IS the thing you read, and the two cannot drift.
#
# Expected variables (all required, all exported by seed-stand.sh):
#   SEED_JOB_NAME             Job name, unique per run
#   SEED_NAMESPACE            namespace holding the release and its creds Secret
#   SEED_IMAGE                the seeder image (the chart's own pin)
#   SEED_STEP                 identity | silver | analytics | all
#   SEED_DEADLINE_SECONDS     wall-clock ceiling for the pod
#   SEED_DB_SECRET            Secret with mariadb-password + clickhouse-password
#   SEED_MARIADB_HOST/_PORT/_USER
#   SEED_IDENTITY_DB          database holding `persons`
#   SEED_ANALYTICS_DB         database holding `metric_definitions`
#   SEED_CLICKHOUSE_HOST/_HTTP_PORT/_USER/_DATABASE
#   SEED_TENANT_ID            tenant every seeded row is scoped to
#   SEED_DEV_USER_EMAIL       persona the dev-lead login resolves to
#   SEED_IDP_SOURCE_TYPE      identity source_type the login rows are written under
#   SEED_CROSS_TENANT         0 | 1 — write the second-tenant refusal fixture
#   SEED_FORCE                0 | 1 — seed a tenant holding foreign person rows
#   SEED_WINDOW_DAYS          activity-window length, empty for the seeder's own
#   SEED_ANCHOR_DATE          last day of activity, empty for the seeder's own
#   SEED_PULL_SECRETS         YAML flow sequence of pull secrets, `[]` for none
apiVersion: batch/v1
kind: Job
metadata:
  name: ${SEED_JOB_NAME}
  namespace: ${SEED_NAMESPACE}
  labels:
    app.kubernetes.io/name: insight-seed
    app.kubernetes.io/component: sample-data
spec:
  # A failed seed needs reading, not retrying: every failure it can produce is
  # deterministic (wrong database, wrong tenant, unreachable dependency), and
  # preflight has already refused the ones that are answerable up front.
  backoffLimit: 0
  activeDeadlineSeconds: ${SEED_DEADLINE_SECONDS}
  # Long enough to read the logs of a finished run, short enough not to litter.
  ttlSecondsAfterFinished: 3600
  template:
    metadata:
      labels:
        app.kubernetes.io/name: insight-seed
        app.kubernetes.io/component: sample-data
    spec:
      restartPolicy: Never
      # Suppress the legacy `<SVC>_SERVICE_HOST/PORT` env vars kubelet injects
      # for every Service in the namespace — several collide by name with the
      # seeder's own variables.
      enableServiceLinks: false
      # The same secrets the release's own Jobs use for this image; `[]` on a
      # stand that pulls it anonymously. Without them a private image leaves
      # the pod in ImagePullBackOff.
      imagePullSecrets: ${SEED_PULL_SECRETS}
      containers:
        - name: seed
          image: ${SEED_IMAGE}
          # IfNotPresent so a locally built image can be tried on a local
          # cluster without pushing it to a registry first.
          imagePullPolicy: IfNotPresent
          # The package is installed in the image, so this is a program on PATH
          # — no shell, no working directory, nothing that depends on where the
          # source happens to sit.
          command: [insight-seed]
          args: [${SEED_STEP}]
          env:
            # ── MariaDB ──────────────────────────────────────────────────
            # The app user, not root: the umbrella grants it ALL on the
            # identity database and it owns the product database, so the seed
            # needs no privilege the services do not already hold.
            - name: MARIADB_HOST
              value: "${SEED_MARIADB_HOST}"
            - name: MARIADB_PORT
              value: "${SEED_MARIADB_PORT}"
            - name: MARIADB_USER
              value: "${SEED_MARIADB_USER}"
            - name: MARIADB_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: ${SEED_DB_SECRET}
                  key: mariadb-password
            - name: MARIADB_DB
              value: "${SEED_IDENTITY_DB}"
            # Required by the seeder, and NOT the compose convention: a
            # chart-deployed stand keeps the catalogue tables in the product
            # database rather than one of their own.
            - name: MARIADB_ANALYTICS_DB
              value: "${SEED_ANALYTICS_DB}"
            # ── ClickHouse ───────────────────────────────────────────────
            - name: CLICKHOUSE_HOST
              value: "${SEED_CLICKHOUSE_HOST}"
            - name: CLICKHOUSE_HTTP_PORT
              value: "${SEED_CLICKHOUSE_HTTP_PORT}"
            - name: CLICKHOUSE_USER
              value: "${SEED_CLICKHOUSE_USER}"
            - name: CLICKHOUSE_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: ${SEED_DB_SECRET}
                  key: clickhouse-password
            - name: CLICKHOUSE_DATABASE
              value: "${SEED_CLICKHOUSE_DATABASE}"
            # ── Seed semantics ───────────────────────────────────────────
            - name: TENANT_DEFAULT_ID
              value: "${SEED_TENANT_ID}"
            - name: DEV_USER_EMAIL
              value: "${SEED_DEV_USER_EMAIL}"
            - name: IDP_SOURCE_TYPE
              value: "${SEED_IDP_SOURCE_TYPE}"
            # Off on a cluster stand: a second tenant makes
            # identity-resolution's scheduled projection abort on its
            # tenant-mismatch guard.
            - name: SEED_CROSS_TENANT_FIXTURE
              value: "${SEED_CROSS_TENANT}"
            - name: SEED_FORCE
              value: "${SEED_FORCE}"
            # Empty means "whatever the seeder documents", which is a window
            # ending yesterday. Pin both to reproduce a dataset exactly.
            - name: SEED_DAYS
              value: "${SEED_WINDOW_DAYS}"
            - name: SEED_ANCHOR_DATE
              value: "${SEED_ANCHOR_DATE}"
            # The pod's filesystem dies with it and the manifest is echoed to
            # the log, so this only has to be somewhere writable — the package
            # is installed in the image and its working directory is not.
            - name: SEED_MANIFEST_PATH
              value: /tmp/manifest.json
            # dbt (run by the silver step's gold build) writes target/, logs/
            # and ~/.dbt under whatever it is given; the image owns /ingestion,
            # but keep the scratch paths explicit.
            - name: DBT_TARGET_PATH
              value: /tmp/dbt-target
            - name: DBT_LOG_PATH
              value: /tmp/dbt-logs
            - name: HOME
              value: /tmp
          resources:
            requests:
              cpu: 250m
              memory: 512Mi
            limits:
              cpu: "2"
              memory: 2Gi
