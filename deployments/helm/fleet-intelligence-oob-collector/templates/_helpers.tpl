{{- define "fleet-intelligence-oob-collector.name" -}}
{{- .Chart.Name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.fullname" -}}
{{- if contains .Chart.Name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | quote }}
app.kubernetes.io/name: {{ include "fleet-intelligence-oob-collector.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.selectorLabels" -}}
app.kubernetes.io/name: {{ include "fleet-intelligence-oob-collector.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.image" -}}
{{- printf "%s:%s" .Values.bmcCollector.image.repository (default .Chart.Version .Values.bmcCollector.image.tag) -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.nicoHardwareHealthName" -}}
{{- printf "%s-nico-hardware-health" (include "fleet-intelligence-oob-collector.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.nicoHardwareHealthImage" -}}
{{- printf "%s:%s" .Values.nicoHardwareHealth.image.repository (default .Chart.Version .Values.nicoHardwareHealth.image.tag) -}}
{{- end }}
