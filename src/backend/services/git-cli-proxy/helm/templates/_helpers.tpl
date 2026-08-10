{{- define "insight-git-cli-proxy.fullname" -}}
{{ .Release.Name }}-git-cli-proxy
{{- end }}

{{- define "insight-git-cli-proxy.labels" -}}
app.kubernetes.io/name: git-cli-proxy
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-git-cli-proxy.selectorLabels" -}}
app.kubernetes.io/name: git-cli-proxy
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- /*
Bytes in a Kubernetes quantity like `50Gi`. Only the forms an operator would
write for a volume are accepted; anything else fails loudly rather than being
silently read as zero, which would make the budget guard below vacuous.
*/ -}}
{{- define "insight-git-cli-proxy.quantityBytes" -}}
{{- $q := . | toString -}}
{{- $n := regexFind "^[0-9]+" $q -}}
{{- if not $n -}}
  {{- fail (printf "persistence.size=%q is not a Kubernetes quantity (expected e.g. 50Gi, 512Mi, 1T)" $q) -}}
{{- end -}}
{{- $unit := trimPrefix $n $q -}}
{{- $factors := dict "" 1 "K" 1000 "M" 1000000 "G" 1000000000 "T" 1000000000000 "Ki" 1024 "Mi" 1048576 "Gi" 1073741824 "Ti" 1099511627776 -}}
{{- if not (hasKey $factors $unit) -}}
  {{- fail (printf "persistence.size=%q has an unsupported unit %q (expected one of: Ki Mi Gi Ti K M G T, or none)" $q $unit) -}}
{{- end -}}
{{- mul (int64 $n) (int64 (index $factors $unit)) -}}
{{- end }}

{{- /*
The cache budget must sit below the volume it lives on: the app reclaims to
its own budget, but the VOLUME filling up is enforced by kubelet as pod
eviction — a crash, not a mechanism. Guarded rather than computed, so the
number stays explicit in a GitOps diff instead of moving when someone edits
persistence.size.
*/ -}}
{{- define "insight-git-cli-proxy.validateDiskBudget" -}}
{{- $sizeBytes := int64 (include "insight-git-cli-proxy.quantityBytes" (required "persistence.size is required" .Values.persistence.size)) -}}
{{- $budget := int64 (required "cache.diskBudgetBytes is required" .Values.cache.diskBudgetBytes) -}}
{{- $repoCap := int64 (required "cache.maxRepoBytes is required" .Values.cache.maxRepoBytes) -}}
{{- /* Integer comparison only: Helm parses YAML numbers as float64, and a
       budget in the 1e10 range renders in scientific notation once it becomes
       one — which the service then refuses to deserialize. */ -}}
{{- if gt (mul $budget 100) (mul $sizeBytes 90) -}}
  {{- fail (printf "cache.diskBudgetBytes (%d) must be at most 90%% of persistence.size (%d bytes). The remainder is headroom for transient packs and git temp files; without it an ENOSPC mid-pack-write is a failed sync, and a full volume is a pod eviction." $budget $sizeBytes) -}}
{{- end -}}
{{- if lt (mul $budget 100) (mul $sizeBytes 50) -}}
  {{- fail (printf "cache.diskBudgetBytes (%d) is under 50%% of persistence.size (%d bytes) — over half the volume would be unusable. Raise the budget or shrink the volume." $budget $sizeBytes) -}}
{{- end -}}
{{- if gt $repoCap $budget -}}
  {{- fail (printf "cache.maxRepoBytes (%d) exceeds cache.diskBudgetBytes (%d): a repository admitted at the per-repo cap could not fit in the cache at all." $repoCap $budget) -}}
{{- end -}}
{{- end }}
