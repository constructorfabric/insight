# One-shot Job that creates ONE connector's bronze tables on a Kubernetes stand.
#
# Bronze has exactly one creator — destination-clickhouse, handed a catalogue
# built from the connector's own `discover`. On compose that runs on the
# caller's Docker (scripts/bootstrap-db/create-connector-tables.sh); a cluster
# has no Docker, so seed-stand.sh builds the catalogue locally and applies it
# here instead. Both paths run the same image over the same catalogue, so a
# table created either way is the one a real sync creates.
#
# Rendered by `seed-stand.sh` with envsubst and applied to the cluster, on the
# same contract as seed-job.yaml.tpl: every variable is resolved from the stand
# and the render refuses when one is empty, so this file is the reference
# manifest.
#
# Expected variables (all required, all exported by seed-stand.sh):
#   BRONZE_JOB_NAME           Job name, unique per run and connector
#   BRONZE_CONNECTOR          connector name, for labels and the log
#   BRONZE_CATALOG_CONFIGMAP  ConfigMap holding configured_catalog.json + traces.jsonl
#   BRONZE_DESTINATION_IMAGE  destination-clickhouse, pinned by bootstrap-db/pins.env
#   SEED_NAMESPACE            namespace holding the release and its creds Secret
#   SEED_IMAGE                the seeder image — renders the destination config
#   SEED_DEADLINE_SECONDS     wall-clock ceiling for the pod
#   SEED_DB_SECRET            Secret with clickhouse-password
#   SEED_CLICKHOUSE_HOST/_HTTP_PORT/_USER/_DATABASE
#   SEED_PULL_SECRETS         YAML flow sequence of pull secrets, `[]` for none
apiVersion: batch/v1
kind: Job
metadata:
  name: ${BRONZE_JOB_NAME}
  namespace: ${SEED_NAMESPACE}
  labels:
    app.kubernetes.io/name: insight-seed
    app.kubernetes.io/component: bronze-tables
    insight.dev/connector: ${BRONZE_CONNECTOR}
spec:
  # Same reasoning as the seeder's Job: every failure here is deterministic —
  # an unreachable ClickHouse, a catalogue the destination rejects — so a
  # retry reproduces it and only hides which attempt to read.
  backoffLimit: 0
  activeDeadlineSeconds: ${SEED_DEADLINE_SECONDS}
  ttlSecondsAfterFinished: 3600
  template:
    metadata:
      labels:
        app.kubernetes.io/name: insight-seed
        app.kubernetes.io/component: bronze-tables
        insight.dev/connector: ${BRONZE_CONNECTOR}
    spec:
      restartPolicy: Never
      enableServiceLinks: false
      imagePullSecrets: ${SEED_PULL_SECRETS}
      # The rendered config is written by one container and read by another
      # under its own baked-in user; fsGroup is what makes the emptyDir
      # readable to both without either image having to run as a chosen uid.
      securityContext:
        fsGroup: 1000
      volumes:
        - name: catalog
          configMap:
            name: ${BRONZE_CATALOG_CONFIGMAP}
        - name: work
          emptyDir: {}
      initContainers:
        # The destination takes its credentials as a FILE, so the password has
        # to be rendered into one. Rendered here from the Secret the release
        # already holds rather than written into a second object: one copy of
        # the value, and it dies with the pod.
        - name: render-destination-config
          image: ${SEED_IMAGE}
          imagePullPolicy: IfNotPresent
          command: [python3, -c]
          args:
            - |
              import json, os, pathlib

              # json.dumps, not a format string: a password is an arbitrary
              # byte sequence and the destination parses this as JSON.
              config = {
                  "host": os.environ["CLICKHOUSE_HOST"],
                  "port": os.environ["CLICKHOUSE_HTTP_PORT"],
                  "protocol": "http",
                  "database": os.environ["CLICKHOUSE_DATABASE"],
                  "username": os.environ["CLICKHOUSE_USER"],
                  "password": os.environ["CLICKHOUSE_PASSWORD"],
                  "enable_json": False,
              }
              path = pathlib.Path("/work/destination_config.json")
              path.write_text(json.dumps(config))
              path.chmod(0o640)
          env:
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
          volumeMounts:
            - {name: work, mountPath: /work}
          resources:
            requests: {cpu: 50m, memory: 64Mi}
            limits: {cpu: "1", memory: 256Mi}
      containers:
        # WORKAROUND: the destination reads its records from stdin, which a Pod
        # cannot redirect — so the image's own entrypoint is execed under a
        # shell that can. `exec` keeps it PID 1, so a deadline still reaches it.
        - name: destination
          image: ${BRONZE_DESTINATION_IMAGE}
          command: [/bin/sh, -c]
          args:
            - >-
              exec /airbyte/base.sh write
              --config /work/destination_config.json
              --catalog /catalog/configured_catalog.json
              < /catalog/traces.jsonl
          volumeMounts:
            - {name: catalog, mountPath: /catalog, readOnly: true}
            - {name: work, mountPath: /work, readOnly: true}
          resources:
            requests: {cpu: 100m, memory: 512Mi}
            limits: {cpu: "1", memory: 2Gi}
