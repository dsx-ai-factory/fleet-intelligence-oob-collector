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

{{- define "fleet-intelligence-oob-collector.otelGatewayEnrollEndpoint" -}}
{{- printf "%s/api/v1/health/enroll" (trimSuffix "/" .Values.backend.url) -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.otelGatewayBackendEndpoint" -}}
{{- printf "%s/api/v1/health" (trimSuffix "/" .Values.backend.url) -}}
{{- end }}

{{- define "fleet-intelligence-oob-collector.scheduling" -}}
{{- with .nodeSelector }}
nodeSelector:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .tolerations }}
tolerations:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .affinity }}
affinity:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.proxyEnv" -}}
{{- if .Values.proxy.enabled }}
{{- range $name, $value := dict "HTTP_PROXY" .Values.proxy.httpProxy "HTTPS_PROXY" .Values.proxy.httpsProxy "ALL_PROXY" .Values.proxy.allProxy "NO_PROXY" .Values.proxy.noProxy }}
{{- if $value }}
- name: {{ $name }}
  value: {{ $value | quote }}
{{- end }}
{{- end }}
{{- end }}
{{- end }}

{{- define "fleet-intelligence-oob-collector.validate" -}}
{{- if not (or .Values.bmcCollector.enabled .Values.nicoHardwareHealth.enabled) -}}
{{- fail "enable at least one of bmcCollector or nicoHardwareHealth" -}}
{{- end -}}
{{- if and .Values.otelGateway.enabled (not .Values.nicoHardwareHealth.enabled) -}}
{{- fail "otelGateway requires nicoHardwareHealth to be enabled" -}}
{{- end -}}
{{- if and .Values.otelGateway.enabled .Values.telemetry.endpoint -}}
{{- fail "telemetry.endpoint is for an external destination; disable otelGateway to use it" -}}
{{- end -}}
{{- if and .Values.nicoHardwareHealth.enabled (not .Values.otelGateway.enabled) (not .Values.telemetry.endpoint) -}}
{{- fail "telemetry.endpoint is required for OTLP export when otelGateway is disabled" -}}
{{- end -}}
{{- if and .Values.proxy.enabled (not (or .Values.proxy.httpProxy .Values.proxy.httpsProxy .Values.proxy.allProxy)) -}}
{{- fail "proxy.enabled requires at least one proxy URL" -}}
{{- end -}}
{{- $seen := dict -}}
{{- range .Values.inventory.targets -}}
{{- $mac := upper (replace "-" "" (replace ":" "" .mac)) -}}
{{- if hasKey $seen $mac -}}{{- fail "inventory.targets contains a duplicate BMC MAC" -}}{{- end -}}
{{- $_ := set $seen $mac true -}}
{{- $_ := include "fleet-intelligence-oob-collector.targetLabels" (dict "root" $ "target" .) -}}
{{- end -}}
{{- end }}
