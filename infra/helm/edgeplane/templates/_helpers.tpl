{{- define "edgeplane.name" -}}
{{- default "edgeplane" .Values.nameOverride -}}
{{- end -}}

{{- define "edgeplane.fullname" -}}
{{- printf "%s-%s" (include "edgeplane.name" .) .Release.Namespace | trunc 63 | trimSuffix "-" -}}
{{- end -}}
