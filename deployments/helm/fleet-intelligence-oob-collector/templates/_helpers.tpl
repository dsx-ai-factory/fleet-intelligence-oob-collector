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
{{- "nico-hardware-health" }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.nicoHardwareHealthSelectorLabels" -}}
app.kubernetes.io/name: {{ include "fleet-intelligence-oob-collector.nicoHardwareHealthName" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.nicoHardwareHealthImage" -}}
{{- printf "%s:%s" .Values.nicoHardwareHealth.image.repository (default .Chart.Version .Values.nicoHardwareHealth.image.tag) -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.otelGatewayImage" -}}
{{- printf "%s:%s" .Values.otelGateway.image.repository .Values.otelGateway.image.tag -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.otelGatewayName" -}}
{{- printf "%s-otel-gateway" (include "fleet-intelligence-oob-collector.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.otelGatewaySelectorLabels" -}}
app.kubernetes.io/name: {{ include "fleet-intelligence-oob-collector.otelGatewayName" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.otelGatewayEnrollSecretName" -}}
{{- default .Values.bmcCollector.agentTokenSecretName .Values.otelGateway.enroll.existingSecret -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.otelGatewayEnrollEndpoint" -}}
{{- default (printf "%s/api/v1/health/enroll" (trimSuffix "/" .Values.bmcCollector.backendUrl)) .Values.otelGateway.enroll.endpoint -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.otelGatewayBackendEndpoint" -}}
{{- default (printf "%s/api/v1/health" (trimSuffix "/" .Values.bmcCollector.backendUrl)) .Values.otelGateway.backendEndpoint | trimSuffix "/" -}}
{{- end }}
