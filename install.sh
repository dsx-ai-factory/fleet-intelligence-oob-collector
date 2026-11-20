#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Parse the complete installer before reading answers from the terminal.
fleetint_install() {
  set -eEuo pipefail
  export LC_ALL=C
  umask 077
  work_dir=''
  child_pid=''
  REPLY=''
  ui_active=no
  ui_reading=no
  pending_cancel=''
  terminal_state=''
  ui_indent='  '

  ui_leave() {
    if [[ $ui_active == yes ]]; then
      if [[ -n "$terminal_state" ]]; then
        if ! stty "$terminal_state" <&3; then printf '%s\n' 'Could not restore terminal settings; run stty sane.' >&2; fi
        terminal_state=''
      fi
      printf '\033[?2004l\033[0m\033[?25h\033[?1049l'
      ui_active=no
    fi
  }
  fail() { ui_leave; printf '%s\n' "$*" >&2; exit 1; }
  cleanup() { ui_leave; if [[ -n "$work_dir" ]]; then rm -rf -- "$work_dir"; fi; }
  cancel() {
    trap - INT TERM
    if [[ -n "$child_pid" ]]; then
      kill -TERM "$child_pid" 2>/dev/null || :
      wait "$child_pid" 2>/dev/null || :
    fi
    ui_leave
    printf '%s\n' 'Installation cancelled. Any resources already created remain in the cluster.' >&2
    exit "$1"
  }
  request_cancel() {
    if [[ $ui_reading == yes ]]; then
      pending_cancel=$1
    else
      cancel "$1"
    fi
  }
  trap cleanup EXIT
  trap 'request_cancel 130' INT
  trap 'request_cancel 143' TERM
  trap 'ui_leave; printf "%s\n" "Installation stopped after a command failed. Review the diagnostic above, resolve the error, and retry; existing Secrets are preserved." >&2' ERR

  # Waiting on a background child lets the signal traps cancel long Helm operations.
  run() {
    local status=0 output_file
    if [[ $1 == --capture ]]; then
      output_file=$2
      shift 2
      "$@" </dev/null > "$output_file" 2>&1 &
    else
      "$@" </dev/null &
    fi
    child_pid=$!
    wait "$child_pid" || status=$?
    child_pid=''
    return "$status"
  }

  valid_text() { [[ ${#1} -le 1024 ]]; }
  valid_label() { [[ ${#1} -le 63 && $1 =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ ]]; }
  valid_release() { [[ ${#1} -le 53 ]] && valid_label "$1"; }
  valid_secret() {
    local part rest=$1
    [[ ${#rest} -le 253 && $rest != *. ]] || return 1
    while [[ $rest == *.* ]]; do
      part=${rest%%.*}
      valid_label "$part" || return 1
      rest=${rest#*.}
    done
    valid_label "$rest"
  }
  valid_key() { [[ ${#1} -le 253 && $1 =~ ^[a-zA-Z0-9._-]+$ ]]; }
  valid_version() { [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; }
  valid_yes_no() { [[ $1 == yes || $1 == no ]]; }
  valid_workflow() { [[ $1 == basic || $1 == advanced ]]; }
  valid_source() { [[ $1 == existing || $1 == file ]]; }
  valid_kind() { [[ $1 == compute || $1 == nvswitch || $1 == power_shelf || $1 == unknown ]]; }
  valid_target_label() {
    [[ $1 =~ ^[a-zA-Z_][a-zA-Z0-9_]*$ && $label_keys != *"|$1|"* ]] || return 1
    case $1 in
      bmc_endpoint|bmc_ip|collector_type|component_type|driver_version|endpoint_ip|endpoint_key|endpoint_mac|gpu_chassis_serial|gpu_model|gpu_serial|gpu_uuid|machine_id|machine_serial|machine_slot_number|machine_tray_index|machine_name|compute_zone|node_group|nvlink_domain_uuid|oob|power_shelf_id|power_shelf_serial_number|rack_id|serial_number|switch_endpoint|switch_endpoint_role|switch_id|switch_ip|switch_is_primary|switch_serial_number|switch_slot_number|switch_tray_index|system_uuid) return 1 ;;
    esac
  }
  valid_path() { [[ $1 == /* ]]; }
  valid_port() {
    local digits=$1
    [[ $digits =~ ^[0-9]+$ ]] || return 1
    while [[ $digits == 0* && ${#digits} -gt 1 ]]; do digits=${digits#0}; done
    [[ ${#digits} -le 5 ]] && (( 10#$digits >= 1 && 10#$digits <= 65535 )) || return 1
    REPLY=$digits
  }
  valid_ipv4() {
    local part
    local parts=()
    [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
    IFS=. read -r -a parts <<< "$1"
    for part in "${parts[@]}"; do
      [[ ${#part} -le 3 && ! $part =~ ^0[0-9] ]] && (( 10#$part <= 255 )) || return 1
    done
  }
  valid_ipv6() {
    local ip=$1 side part count=0 compressed=no
    local sides=() parts=()
    [[ $ip == *:* && $ip != *%* ]] || return 1
    if [[ $ip == *.* ]]; then
      valid_ipv4 "${ip##*:}" || return 1
      ip=${ip%:*}:0:0
    fi
    if [[ $ip == *::* ]]; then
      compressed=yes
      sides=("${ip%%::*}" "${ip#*::}")
      [[ ${sides[1]} != *::* ]] || return 1
    else
      sides=("$ip")
    fi
    for side in "${sides[@]}"; do
      [[ -n "$side" ]] || continue
      [[ $side != :* && $side != *: ]] || return 1
      IFS=: read -r -a parts <<< "$side"
      for part in "${parts[@]}"; do
        [[ $part =~ ^[0-9a-fA-F]{1,4}$ ]] || return 1
        count=$((count + 1))
      done
    done
    if [[ $compressed == yes ]]; then (( count < 8 )); else (( count == 8 )); fi
  }
  valid_ip() { valid_ipv4 "$1" || valid_ipv6 "$1"; }
  valid_mac() { [[ $1 =~ ^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$ || $1 =~ ^([0-9a-fA-F]{2}-){5}[0-9a-fA-F]{2}$ || $1 =~ ^[0-9a-fA-F]{12}$ ]]; }
  normalize_mac() {
    local raw=${1//[-:]/} i octet result=''
    for ((i=0; i<12; i+=2)); do
      printf -v octet '%02X' "$((16#${raw:i:2}))"
      result+="${result:+:}$octet"
    done
    REPLY=$result
  }
  valid_unique_mac() {
    valid_mac "$1" || return 1
    normalize_mac "$1"
    [[ $seen_macs != *"|$REPLY|"* ]]
  }
  valid_backend() {
    local authority host port=''
    [[ $1 == https://* ]] || return 1
    authority=${1#https://}
    authority=${authority%/}
    if [[ $authority =~ ^\[([^]]+)\](:([0-9]+))?$ ]]; then
      host=${BASH_REMATCH[1]}
      port=${BASH_REMATCH[3]}
      valid_ipv6 "$host" || return 1
    elif [[ $authority =~ ^([a-zA-Z0-9._-]+)(:([0-9]+))?$ ]]; then
      port=${BASH_REMATCH[3]}
    else
      return 1
    fi
    [[ -z "$port" ]] || valid_port "$port"
  }

  ui_emit() {
    local fragment
    printf -v fragment "$@"
    ui_frame+=$fragment
  }
  ui_border() {
    local left=$ui_top_left right=$ui_top_right
    if [[ $1 == bottom ]]; then left=$ui_bottom_left; right=$ui_bottom_right; fi
    ui_emit '%s%s%s%s%s%s\n' "$ui_indent" "$ui_muted" "$left" "$ui_rule" "$right" "$ui_reset"
  }
  ui_row() {
    local text=$1 style=${2:-$ui_reset}
    ui_emit '%s%s%s%s%*s\033[%sG%s%s\033[%sG%s%s%s\n' "$ui_indent" "$ui_muted" "$ui_vertical" "$style" 70 '' "$ui_text_column" "$style" "$text" "$ui_right_column" "$ui_reset$ui_muted" "$ui_vertical" "$ui_reset"
  }
  ui_paragraph() {
    local text=$1 style=${2:-$ui_reset} boundary lines=0
    while (( ${#text} > 68 )); do
      boundary=68
      if (( lines == 1 )); then boundary=65; fi
      while [[ ${text:boundary:1} == [$'\200'-$'\277'] ]] && (( boundary > 0 )); do
        boundary=$((boundary - 1))
      done
      if (( lines == 1 )); then
        ui_emit '%s%s%s...%s\n' "$ui_indent" "$style" "${text:0:boundary}" "$ui_reset"
        return
      fi
      ui_emit '%s%s%s%s\n' "$ui_indent" "$style" "${text:0:boundary}" "$ui_reset"
      text=${text:boundary}
      lines=$((lines + 1))
    done
    ui_emit '%s%s%s%s\n' "$ui_indent" "$style" "$text" "$ui_reset"
  }
  ui_flush() {
    # Draw a complete frame in one write; erase stale line tails without blanking the screen.
    printf '%s\033[?25l%s\033[J' "$ui_reset" "${ui_frame//$'\n'/$'\033[K\n'}"
  }
  install_status() {
    local accent=$ui_accent reset=''
    if [[ $1 == warning ]]; then verification_warnings=yes; fi
    if [[ $install_ui == yes ]]; then
      if [[ -n $accent ]]; then
        reset=$'\033[0m'
        case $1 in failed) accent=$'\033[91m' ;; warning) accent=$'\033[93m' ;; esac
      fi
      printf '\n  %s[%s]%s %s\n' "$accent" "$1" "$reset" "$2"
      printf '[%s] %s\n' "$1" "$2" >> "$work_dir/kubernetes.log"
    else
      printf '\n[%s] %s\n' "$1" "$2"
    fi
  }
  show_verified_checks() {
    local check mark='[OK]' accent='' reset=''
    if [[ $install_ui == yes ]]; then
      accent=$ui_accent
      if [[ $ui_glyphs == unicode ]]; then mark=$ui_complete; fi
      if [[ -n $accent ]]; then reset=$'\033[0m'; fi
    fi
    printf '\n  %s\n' 'Verified checks'
    for check in "${verified_checks[@]}"; do
      printf '    %s%s%s  %s\n' "$accent" "$mark" "$reset" "$check"
    done
  }
  terminal_line() {
    local line=$1 ansi=$'\033''\[[0-?]*[ -/]*[@-~]'
    # Treat cluster output as text even if it contains terminal commands.
    while [[ $line =~ $ansi ]]; do line=${line/"${BASH_REMATCH[0]}"/}; done
    line=${line//$'\t'/ }
    REPLY=${line//[[:cntrl:]]/}
  }
  show_diagnostic() {
    local line rows=0
    while IFS= read -r line || [[ -n $line ]]; do
      terminal_line "$line"
      printf '%.400s\n' "$REPLY"
      rows=$((rows + 1))
      if (( rows == 24 )); then
        printf '%s\n' 'Additional output omitted.'
        break
      fi
    done < "$1"
  }
  remember_log() {
    [[ $install_ui == yes ]] || return 0
    {
      printf '\n%s\n' "$2"
      if [[ -s $3 ]]; then show_diagnostic "$3"; else printf '%s\n' 'No entries returned.'; fi
    } >> "$work_dir/$1.log"
  }
  run_logged() {
    local title=$1 status=0
    shift
    if [[ $install_ui != yes ]]; then run "$@"; return; fi
    run --capture "$work_dir/command-output" "$@" || status=$?
    show_diagnostic "$work_dir/command-output"
    remember_log kubernetes "$title" "$work_dir/command-output"
    return "$status"
  }
  diagnostic() {
    local source=$1 title=$2
    shift 2
    printf '\n%s\n' "$title"
    if ! run "$@" > "$work_dir/diagnostic" 2>&1; then
      printf '%s\n' 'Could not retrieve this diagnostic.'
    fi
    if [[ -s "$work_dir/diagnostic" ]]; then
      show_diagnostic "$work_dir/diagnostic"
    else
      printf '%s\n' 'No entries returned.'
    fi
    remember_log "$source" "$title" "$work_dir/diagnostic"
  }
  release_logs() {
    local selector=$1 container=$2
    shift 2
    run "${kube[@]}" logs --selector "$selector" --container "$container" \
      --tail=16 --limit-bytes=4096 --request-timeout=5s --pod-running-timeout=5s "$@"
  }
  deployment_failed() {
    local uid event_count=0 pods
    install_status failed "$1"
    diagnostic kubernetes 'Pods for this release' "${kube[@]}" get pods --selector "$release_selector" --request-timeout=5s
    pods=$(< "$work_dir/diagnostic")
    if run "${kube[@]}" get pods,replicasets --selector "$release_selector" --request-timeout=5s \
        -o 'jsonpath={range .items[*]}{.metadata.uid}{"\n"}{end}' > "$work_dir/workload-uids" 2> "$work_dir/diagnostic"; then
      while IFS= read -r uid || [[ -n $uid ]]; do
        [[ $uid =~ ^[a-zA-Z0-9-]+$ ]] || continue
        diagnostic kubernetes 'Kubernetes warning events' "${kube[@]}" get events --request-timeout=5s \
          --field-selector "involvedObject.uid=$uid,type=Warning"
        event_count=$((event_count + 1))
        if (( event_count >= 8 )); then break; fi
      done < "$work_dir/workload-uids"
    else
      printf '\n%s\n' 'Could not list workloads for Kubernetes warning events.'
      show_diagnostic "$work_dir/diagnostic"
      remember_log kubernetes 'Could not list workloads for Kubernetes warning events' "$work_dir/diagnostic"
    fi
    diagnostic collector 'Collector logs' release_logs "$collector_selector" collector
    diagnostic nico 'NICo configuration logs' release_logs "$nico_selector" generate-nico-config
    diagnostic nico 'NICo health logs' release_logs "$nico_selector" nico-hardware-health
    diagnostic gateway 'Telemetry gateway logs' release_logs "$gateway_selector" otel-gateway
    if [[ $pods == *CrashLoopBackOff* || $pods == *' Error '* ]]; then
      diagnostic collector 'Previous collector logs' release_logs "$collector_selector" collector --previous
    fi
    ui_verification "$1"
    show_verified_checks
    fail 'Installation could not be verified. Resources remain in the cluster for troubleshooting.'
  }
  verify_rollout() {
    local component=$1 selector=$2 deployment
    run "${kube[@]}" get deployment --selector "$selector" -o name --request-timeout=10s > "$work_dir/deployment" ||
      deployment_failed "Could not find the $component Deployment."
    deployment=$(< "$work_dir/deployment")
    [[ $deployment =~ ^deployment(\.apps)?/[a-z0-9.-]+$ ]] ||
      deployment_failed "Expected one $component Deployment in this release."
    run_logged "$component rollout status" "${kube[@]}" rollout status "$deployment" --timeout=2m ||
      deployment_failed "$component rollout did not finish."
    REPLY=$deployment
  }
  inventory_receipt() {
    local line message log_pattern='^([0-9]{4}-[^[:space:]]+[[:space:]]+)?(INFO|WARN|ERROR)[[:space:]]+fleet_intelligence_oob:[[:space:]]+(.*)$'
    latest_inventory_warning=''
    while IFS= read -r line || [[ -n $line ]]; do
      terminal_line "$line"
      [[ $REPLY =~ $log_pattern ]] || continue
      if [[ ${BASH_REMATCH[2]} != INFO ]]; then latest_inventory_warning=$REPLY; fi
      message=${BASH_REMATCH[3]}
      case $message in
        'first inventory upload accepted in this cycle'|'first inventory upload accepted in this cycle '*) return 0 ;;
        'inventory mini-batch export completed '*|'inventory backend sync completed '*)
          # Older charts report accepted counts or log a fully successful nonempty batch.
          if [[ $message =~ [[:space:]](accepted_upload_count|accepted_count|exported_count)=[1-9][0-9]*([[:space:]]|$) ]] ||
             [[ $message =~ ^inventory\ mini-batch\ export\ completed\ exportable_count=[1-9][0-9]*([[:space:]]|$) ]]; then
            return 0
          fi
          ;;
      esac
    done < "$1"
    return 1
  }
  wait_for_inventory() {
    local deployment=$1 deadline=$((SECONDS + 360)) attempt remaining request_timeout error
    local latest_inventory_warning='' shown_warning=''
    for ((attempt=1; attempt<=73; attempt++)); do
      remaining=$((deadline - SECONDS))
      (( remaining > 0 )) || return 1
      request_timeout=$((remaining < 10 ? remaining : 10))
      if run "${kube[@]}" logs "$deployment" --container collector --tail=500 --limit-bytes=131072 \
          "--request-timeout=${request_timeout}s" "--pod-running-timeout=${request_timeout}s" \
          > "$work_dir/inventory-logs" 2> "$work_dir/log-error"; then
        if inventory_receipt "$work_dir/inventory-logs"; then
          printf '%.400s\n' "$REPLY"
          if [[ $install_ui == yes ]]; then printf '%s\n' "$REPLY" >> "$work_dir/collector.log"; fi
          return 0
        fi
        if [[ -n $latest_inventory_warning && $latest_inventory_warning != "$shown_warning" ]]; then
          install_status warning 'Collector still has no confirmed inventory upload'
          printf '%.400s\n' "$latest_inventory_warning"
          if [[ $install_ui == yes ]]; then printf '%.400s\n' "$latest_inventory_warning" >> "$work_dir/collector.log"; fi
          shown_warning=$latest_inventory_warning
        fi
      else
        error=$(< "$work_dir/log-error")
        if [[ $error == *' (Forbidden)'* || $error == *' (Unauthorized)'* ]]; then
          show_diagnostic "$work_dir/log-error"
          remember_log collector 'Could not read inventory logs' "$work_dir/log-error"
          return 1
        fi
        if (( attempt == 1 )); then
          printf '%s\n' 'Collector logs unavailable. Retrying within the verification window.'
          show_diagnostic "$work_dir/log-error"
          remember_log collector 'Could not read inventory logs' "$work_dir/log-error"
        fi
      fi
      remaining=$((deadline - SECONDS))
      (( remaining > 0 && attempt < 73 )) || return 1
      run sleep "$((remaining < 5 ? remaining : 5))" || return 1
    done
  }
  report_workload_warnings() {
    local source=$1 component=$2 selector=$3 container=$4 line heading=no
    if ! release_logs "$selector" "$container" > "$work_dir/recent-logs" 2>&1; then
      install_status warning "Could not check recent $component logs."
      show_diagnostic "$work_dir/recent-logs"
      remember_log "$source" "Could not check recent $component logs" "$work_dir/recent-logs"
      return 0
    fi
    remember_log "$source" "Recent $component logs" "$work_dir/recent-logs"
    while IFS= read -r line || [[ -n $line ]]; do
      terminal_line "$line"
      if [[ $REPLY =~ ^([^[:space:]]+[[:space:]]+)?(WARN|ERROR|FATAL|warn|error|fatal|Error)([[:space:]:]|$) ||
            $REPLY =~ \"level\":[[:space:]]*\"(warn|error|fatal)\" ]]; then
        if [[ $heading == no ]]; then
          install_status warning "Recent $component log messages"
          heading=yes
        fi
        printf '%.400s\n' "$REPLY"
      fi
    done < "$work_dir/recent-logs"
  }
  ui_screen() {
    local title=$1 hint='One step at a time. Your settings stay on this machine.' step index style marker
    local steps=(Cluster Health Targets Secrets Access Review)
    step=${ui_section:0:1}
    if [[ $workflow == basic ]]; then steps=(Setup Target Access Review); fi
    case $title in
      'Installation workflow'*) title='Choose an installation workflow'; hint='Choose how much of the setup you want to customize.' ;;
      'Kubernetes context'*)
        title='Kubernetes context'
        hint='List contexts with kubectl config get-contexts.'
        if [[ $workflow == basic ]]; then hint='No current context found. List with kubectl config get-contexts.'; fi
        ;;
      Namespace) title='Kubernetes namespace'; hint='Use a dedicated namespace to keep the workloads together.' ;;
      'Helm release name') hint='Use this name to manage and upgrade the Helm release.' ;;
      'Checking for the latest chart release') hint='Looking up the latest stable release in NGC. Ctrl-C cancels.' ;;
      'NGC chart version')
        title='Choose a chart version'
        if [[ -n ${FLEETINT_INSTALL_CHART_VERSION:-} ]]; then
          hint='Press Enter to keep the default or enter another exact version.'
        elif [[ -n $latest_version ]]; then
          hint='Press Enter for the latest stable release or enter an exact version.'
        else
          hint='Latest release lookup failed. Enter an exact NGC chart version.'
        fi
        ;;
      'Fleet Intelligence HTTPS'*) title='Connect to Fleet Intelligence'; hint='Use the production endpoint or your own HTTPS backend origin.' ;;
      'BMC IP address'*) title='BMC IP address'; hint='Use the BMC IPv4 or IPv6 address without a scheme or port.' ;;
      'BMC MAC address'*) title='BMC MAC address'; hint='Enter the MAC of the management controller itself.' ;;
      'Kind: compute'*) title='Choose the hardware type'; hint='This tells Fleet Intelligence what kind of device you collect.' ;;
      'Redfish HTTPS port') hint='Use 443 unless this BMC serves Redfish on a different port.' ;;
      'Display name'*) title='Display name'; hint='Use a name that helps you identify this BMC in Fleet Intelligence.' ;;
      'Credentials profile'*) title='Credentials profile'; hint='Use a JSON profile, or leave blank for a complete MAC override.' ;;
      'Compute zone for this BMC'*) title='Compute zone for this BMC'; hint='Leave blank to use the deployment compute zone.' ;;
      'Node group for this BMC'*) title='Node group for this BMC'; hint='Leave blank to use the deployment node group.' ;;
      'Label key'*) title='Target label key'; hint='Use a key like rack. Leave blank to finish adding labels.' ;;
      'Label value for '*) hint='Enter the value of this target label.' ;;
      'Does NICo already monitor'*) title='Does NICo already monitor these BMCs?'; hint='The bundled NICo collects health metrics and events for your BMCs.' ;;
      'Add another BMC?'*) title='Add another BMC?'; hint='For a first install, verify one BMC before adding more.' ;;
      'Secret source'*) title='Choose a Secret source'; hint='Use a private file or an existing Kubernetes Secret.' ;;
      'Install with these settings?'*) title='Ready to install?'; hint='Install the collector, NICo hardware health, and telemetry gateway.' ;;
      'Review the installation settings') hint='Check the cluster, hardware, and Secret references before installing.' ;;
    esac
    ui_frame=$'\033[H\n'
    ui_emit '%s%s%s%s  %sNVIDIA / FLEET INTELLIGENCE%s\n' "$ui_indent" "$ui_accent" "$ui_logo_top" "$ui_reset" "$ui_bold" "$ui_reset"
    ui_emit '%s%s%s%s  Out-of-band collector\n' "$ui_indent" "$ui_accent" "$ui_logo_middle" "$ui_reset"
    ui_emit '%s%s%s%s  %sHardware visibility, beyond the host.%s\n\n' "$ui_indent" "$ui_accent" "$ui_logo_bottom" "$ui_reset" "$ui_muted" "$ui_reset"
    if [[ $step == [1-9] ]]; then
      ui_emit '%s' "$ui_indent"
      for ((index=0; index<${#steps[@]}; index++)); do
        style=$ui_muted marker=$ui_pending
        if (( index + 1 < step )); then style=$ui_accent; marker=$ui_complete; fi
        if (( index + 1 == step )); then style=$ui_bold; marker=$ui_current; fi
        ui_emit '%s%s %s%s  ' "$style" "$marker" "${steps[$index]}" "$ui_reset"
      done
      ui_emit '\n\n'
    fi
    ui_paragraph "$ui_section" "$ui_muted"
    ui_paragraph "$title" "$ui_bold"
    ui_paragraph "$hint" "$ui_muted"
    ui_emit '\n'
  }
  ui_read() {
    local count=$1 destination=$2 once=${3-} status
    # Bash 3.2 reports an idle timeout the same way it reports EOF.
    if (( BASH_VERSINFO[0] < 4 )); then
      if [[ $once == once ]]; then
        IFS= read -r -s -n "$count" -t 1 "$destination" <&3
      else
        IFS= read -r -s -n "$count" "$destination" <&3
      fi
      return
    fi
    while :; do
      status=0
      ui_reading=yes
      if [[ -z $pending_cancel ]]; then
        # A signal trap may return to a blocked read, so bound the wait.
        IFS= read -r -s -n "$count" -t 1 "$destination" <&3 || status=$?
      fi
      ui_reading=no
      # Bash restores read's saved tty modes when the builtin returns.
      # Restore the original terminal only after that cleanup has finished.
      if [[ -n $pending_cancel ]]; then cancel "$pending_cancel"; fi
      if (( status <= 128 )) || [[ $once == once ]]; then return "$status"; fi
    done
  }
  ui_key() {
    local suffix=''
    REPLY=''
    if ! ui_read 1 REPLY; then
      if [[ ${1-} == close ]]; then REPLY=escape; return; fi
      cancel 130
    fi
    if [[ $REPLY == $'\033' ]]; then
      if ui_read 2 suffix once; then
        case $suffix in
          '[A'|'OA') REPLY=up ;;
          '[B'|'OB') REPLY=down ;;
          '[C'|'OC') REPLY=right ;;
          '[D'|'OD') REPLY=left ;;
          *) REPLY=escape ;;
        esac
      else
        REPLY=escape
      fi
    fi
  }
  ui_character_before() {
    REPLY=$(($1 - 1))
    while (( REPLY > 0 )) && [[ ${ui_value:REPLY:1} == [$'\200'-$'\277'] ]]; do
      REPLY=$((REPLY - 1))
    done
  }
  ui_character_after() {
    REPLY=$(($1 + 1))
    while (( REPLY < ${#ui_value} )) && [[ ${ui_value:REPLY:1} == [$'\200'-$'\277'] ]]; do
      REPLY=$((REPLY + 1))
    done
  }
  ui_field_draw() {
    local start=0 end prefix suffix visible
    # Bound by bytes so even wide UTF-8 characters fit inside the field.
    if (( ui_cursor > 60 )); then start=$((ui_cursor - 60)); fi
    while [[ ${ui_value:start:1} == [$'\200'-$'\277'] ]]; do start=$((start + 1)); done
    end=$((start + 66))
    if (( end > ${#ui_value} )); then end=${#ui_value}; fi
    while [[ ${ui_value:end:1} == [$'\200'-$'\277'] ]]; do end=$((end - 1)); done
    visible=${ui_value//[[:cntrl:]]/?}
    prefix=${visible:start:ui_cursor-start}
    suffix=${visible:ui_cursor:end-ui_cursor}
    printf '%s\033[?25l\r%s%s%s%s%70s\033[%sG%s%s%s\033[%sG%s' "$ui_reset" "$ui_indent" "$ui_reset$ui_muted" "$ui_vertical" "$ui_input" '' "$ui_right_column" "$ui_reset$ui_muted" "$ui_vertical" "$ui_reset" "$ui_text_column" "$ui_input"
    if (( start > 0 )); then printf '<'; else printf ' '; fi
    printf '%s\0337%s' "$prefix" "$suffix"
    if (( end < ${#ui_value} )); then printf '\033[%sG>' "$((ui_text_column + 67))"; fi
    # Let the terminal measure the prefix, including wide and combining characters.
    printf '\0338\033[?25h'
  }
  ui_read_field() {
    local ui_value='' ui_cursor=0 key sequence part paste=no
    ui_field_draw
    while :; do
      key=''
      ui_read 1 key || fail 'Installation cancelled because terminal input ended.'
      if [[ $key == $'\033' ]]; then
        sequence=''
        while (( ${#sequence} < 8 )); do
          part=''
          ui_read 1 part once || break
          sequence+=$part
          [[ $part == [a-zA-Z~] ]] && break
        done
        case $sequence in
          '[D'|'OD') key=$'\002' ;;
          '[C'|'OC') key=$'\006' ;;
          '[H'|'OH'|'[1~'|'[7~') key=$'\001' ;;
          '[F'|'OF'|'[4~'|'[8~') key=$'\005' ;;
          '[3~') key=$'\004' ;;
          '[200~') paste=yes; continue ;;
          '[201~') paste=no; continue ;;
          '') cancel 130 ;;
          *) continue ;;
        esac
      fi
      if [[ $paste == yes ]]; then
        key=${key:-$'\n'}
        ui_value="${ui_value:0:ui_cursor}$key${ui_value:ui_cursor}"
        ui_cursor=$((ui_cursor + ${#key}))
      else
        case $key in
          '') REPLY=$ui_value; printf '%s\033[4B\r' "$ui_reset"; return ;;
          $'\001') ui_cursor=0 ;;
          $'\005') ui_cursor=${#ui_value} ;;
          $'\002') if (( ui_cursor > 0 )); then ui_character_before "$ui_cursor"; ui_cursor=$REPLY; fi ;;
          $'\006') if (( ui_cursor < ${#ui_value} )); then ui_character_after "$ui_cursor"; ui_cursor=$REPLY; fi ;;
          $'\177'|$'\010')
            if (( ui_cursor > 0 )); then
              ui_character_before "$ui_cursor"
              ui_value="${ui_value:0:REPLY}${ui_value:ui_cursor}"; ui_cursor=$REPLY
            fi ;;
          $'\004')
            if (( ui_cursor < ${#ui_value} )); then
              ui_character_after "$ui_cursor"
              ui_value="${ui_value:0:ui_cursor}${ui_value:REPLY}"
            fi ;;
          $'\025') ui_value=${ui_value:ui_cursor}; ui_cursor=0 ;;
          $'\013') ui_value=${ui_value:0:ui_cursor} ;;
          *[[:cntrl:]]*) continue ;;
          *) ui_value="${ui_value:0:ui_cursor}$key${ui_value:ui_cursor}"; ui_cursor=$((ui_cursor + ${#key})) ;;
        esac
      fi
      ui_field_draw
    done
  }
  ui_menu() {
    local label=$1 default=$2 validator=$3 from_env=$4 selected=0 index
    local options=() descriptions=() details=()
    case $validator in
      valid_workflow)
        options=(basic advanced); descriptions=('Basic (recommended)' 'Advanced')
        details=('Start with one BMC and recommended deployment settings.' 'Configure custom settings, Secrets, and multiple BMCs.')
        ;;
      valid_yes_no)
        options=(no yes); descriptions=('No' 'Yes')
        details=('Keep things as they are.' 'Continue with the settings you selected.')
        case $label in
          'Does NICo already monitor'*) details=('Recommended for a first install. Use bundled NICo and gateway.' 'Get the manual steps for keeping your existing NICo deployment.') ;;
          'Add another BMC?'*) details=('Continue to BMC credentials and enrollment.' 'Enter the settings for one more BMC.') ;;
          'Install with these settings?'*) details=('Exit without making changes.' 'Apply these settings and verify the deployment.') ;;
        esac
        ;;
      valid_source)
        options=(existing file); descriptions=('Existing Kubernetes Secret' 'Private file on this machine')
        details=('Works with manually created, Vault, or OpenBao-managed Secrets.' 'Create a missing Secret. Existing Secrets are preserved.')
        ;;
      valid_kind)
        options=(unknown compute nvswitch power_shelf)
        descriptions=('Unspecified device' 'Compute node' 'NVSwitch' 'Power shelf')
        details=('Keep the hardware classification unspecified.' 'A server BMC for host hardware inventory and GPU attestation.' 'A BMC managing an NVSwitch device.' 'A BMC managing a power shelf.')
        ;;
    esac
    for ((index=0; index<${#options[@]}; index++)); do
      if [[ ${options[$index]} == "$default" ]]; then selected=$index; fi
    done
    while :; do
      ui_screen "$label"
      if [[ -n $from_env ]]; then
        ui_paragraph "From environment: $from_env" "$ui_muted"
      fi
      ui_border top
      for ((index=0; index<${#options[@]}; index++)); do
        if (( index == selected )); then
          ui_row "$ui_pointer $((index + 1))  ${descriptions[$index]}" "$ui_selected"
        else
          ui_row "  $((index + 1))  ${descriptions[$index]}"
        fi
      done
      ui_border bottom
      ui_emit '\n'
      ui_paragraph "${details[$selected]}" "$ui_muted"
      ui_emit '\n'
      ui_paragraph 'Arrows or j/k: move. Enter or number: select. Esc: cancel.'
      ui_flush
      ui_key
      case $REPLY in
        up|k) selected=$(((selected + ${#options[@]} - 1) % ${#options[@]})) ;;
        down|j) selected=$(((selected + 1) % ${#options[@]})) ;;
        '') REPLY=${options[$selected]}; return ;;
        [1-9])
          if (( REPLY <= ${#options[@]} )); then REPLY=${options[$((REPLY - 1))]}; return; fi
          ;;
        escape|q) cancel 130 ;;
      esac
    done
  }
  review_add() {
    local text=$1 boundary
    while (( ${#text} > 68 )); do
      boundary=68
      # Keep UTF-8 continuation bytes with the character on the next line.
      while [[ ${text:boundary:1} == [$'\200'-$'\277'] ]] && (( boundary > 0 )); do
        boundary=$((boundary - 1))
      done
      (( boundary > 0 )) || boundary=68
      review_lines[${#review_lines[@]}]=${text:0:boundary}
      text=${text:boundary}
    done
    review_lines[${#review_lines[@]}]=$text
  }
  ui_review() {
    local page=0 per_page=7 pages index start end
    pages=$(((${#review_lines[@]} + per_page - 1) / per_page))
    while :; do
      ui_screen 'Review the installation settings'
      start=$((page * per_page))
      end=$((start + per_page))
      ui_border top
      for ((index=start; index<end && index<${#review_lines[@]}; index++)); do
        ui_row "${review_lines[$index]}"
      done
      ui_border bottom
      ui_emit '\n%sPage %s/%s. n/p: next/previous. Enter: continue. Esc: cancel.\n' "$ui_indent" "$((page + 1))" "$pages"
      ui_flush
      ui_key
      case $REPLY in
        n|down|j) page=$(((page + 1) % pages)) ;;
        p|up|k) page=$(((page + pages - 1) % pages)) ;;
        '') if (( page + 1 < pages )); then page=$((page + 1)); else return 0; fi ;;
        escape|q) cancel 130 ;;
      esac
    done
  }

  ui_verification() {
    [[ $install_ui == yes ]] || return 0
    local problem=${1-} rows columns margin ui_indent ui_text_column ui_right_column
    local view=checks source=0 page=0 pages=1 per_page=7 index start end check line mark=$ui_complete
    local title='Verification complete' title_color=$ui_accent warning_color=''
    local sources=(kubernetes collector nico gateway)
    local labels=(Kubernetes Collector NICo Gateway)
    local review_lines=()

    if ! read -r rows columns < <(stty size <&3) ||
       ! [[ $rows =~ ^[0-9]+$ && $columns =~ ^[0-9]+$ ]] ||
       (( rows < 24 || columns < 80 )); then
      printf '%s\n' 'Terminal too small for verification panes. Results are in the terminal output.'
      return 0
    fi
    margin=$(((columns - 72) / 2))
    printf -v ui_indent '%*s' "$margin" ''
    ui_text_column=$((margin + 3)) ui_right_column=$((margin + 72))
    if ! terminal_state=$(stty -g <&3); then
      printf '%s\n' 'Could not reopen the terminal UI. Results are in the terminal output.'
      return 0
    fi
    ui_active=yes
    if ! stty -echo -icanon min 1 time 0 <&3; then
      ui_leave
      printf '%s\n' 'Could not reopen the terminal UI. Results are in the terminal output.'
      return 0
    fi
    if [[ -n $problem ]]; then
      title='Verification needs attention'
      if [[ -n $ui_accent ]]; then title_color=$'\033[91m'; fi
    fi
    if [[ -n $ui_accent ]]; then warning_color=$'\033[93m'; fi
    if [[ $ui_glyphs == ascii ]]; then mark='[OK]'; fi

    printf '\033[?1049h%s\033[2J' "$ui_reset"
    while :; do
      ui_frame=$'\033[H\n'
      ui_paragraph 'NVIDIA / FLEET INTELLIGENCE' "$ui_bold"
      ui_paragraph "$title" "$title_color"
      ui_paragraph "Release: $release" "$ui_muted"
      ui_emit '\n'
      if [[ $view == checks ]]; then
        ui_emit '%s%s[1 Checks]%s   2 Logs\n\n' "$ui_indent" "$ui_selected" "$ui_reset"
        ui_border top
        for check in "${verified_checks[@]}"; do
          ui_row "$ui_accent$mark$ui_reset  $check"
        done
        ui_border bottom
        ui_emit '\n'
        if [[ -n $problem ]]; then
          ui_paragraph "$problem" "$title_color"
        else
          if [[ $verification_warnings == yes ]]; then
            ui_paragraph 'Recent warnings are available in Logs.' "$warning_color"
          fi
          ui_paragraph 'Still to verify in Fleet Intelligence' "$ui_bold"
          ui_paragraph 'Confirm fresh inventory and health telemetry for each BMC.' "$ui_muted"
        fi
      else
        ui_emit '%s1 Checks   %s[2 Logs]%s\n\n' "$ui_indent" "$ui_selected" "$ui_reset"
        ui_paragraph "${labels[$source]} logs" "$ui_accent"
        ui_emit '%s' "$ui_indent"
        for ((index=0; index<${#labels[@]}; index++)); do
          if (( index == source )); then
            ui_emit '%s[%s]%s  ' "$ui_selected" "${labels[$index]}" "$ui_reset"
          else
            ui_emit '%s  ' "${labels[$index]}"
          fi
        done
        ui_emit '\n'
        review_lines=()
        if [[ -s "$work_dir/${sources[$source]}.log" ]]; then
          while IFS= read -r line || [[ -n $line ]]; do
            if [[ -n $line ]]; then review_add "$line"; fi
          done < "$work_dir/${sources[$source]}.log"
        fi
        if [[ -z ${review_lines[0]-} ]]; then review_lines=('No log entries captured for this source.'); fi
        pages=$(((${#review_lines[@]} + per_page - 1) / per_page))
        start=$((page * per_page)) end=$(((page + 1) * per_page))
        ui_border top
        for ((index=start; index<end && index<${#review_lines[@]}; index++)); do
          ui_row "${review_lines[$index]}"
        done
        ui_border bottom
        ui_paragraph "Page $((page + 1))/$pages. Left/Right: source. Up/Down: page." "$ui_muted"
      fi
      ui_emit '\n'
      ui_paragraph '1: Checks. 2: Logs. Enter: close.' "$ui_muted"
      ui_flush
      ui_key close
      case $REPLY in
        1) view=checks ;;
        2) view=logs ;;
        left|right)
          if [[ $view == logs ]]; then
            if [[ $REPLY == left ]]; then source=$(((source + ${#sources[@]} - 1) % ${#sources[@]}))
            else source=$(((source + 1) % ${#sources[@]})); fi
            page=0
          fi
          ;;
        up|down|p|n)
          if [[ $view == logs ]]; then
            if [[ $REPLY == up || $REPLY == p ]]; then
              if (( page > 0 )); then page=$((page - 1)); fi
            elif (( page + 1 < pages )); then
              page=$((page + 1))
            fi
          fi
          ;;
        ''|escape|q) ui_leave; return ;;
      esac
    done
  }

  ask() {
    local _dest=$1 _label=$2 _default=${3-} _optional=${4-no} _validator=${5-} _answer
    local _env=${6-} _from_env='' _basic=${7-prompt}
    if [[ -n $_env && -n ${!_env-} ]]; then
      _default=${!_env}
      [[ $_default != *[[:cntrl:]]* ]] || fail "Invalid value in $_env. Correct or unset it and rerun the installer."
      _default=${_default#"${_default%%[![:space:]]*}"}
      _default=${_default%"${_default##*[![:space:]]}"}
      if [[ -z $_default ]] || { [[ -n $_validator ]] && ! "$_validator" "$_default"; }; then
        fail "Invalid value in $_env. Correct or unset it and rerun the installer."
      fi
      _from_env=$_env
    fi
    if [[ $workflow == basic && ( $_basic == default || ( $_basic == prefilled && -n $_default ) ) ]]; then
      if [[ -n $_from_env ]]; then environment_defaults=yes; fi
      printf -v "$_dest" '%s' "$_default"
      return
    fi
    while :; do
      if [[ $ui_active == yes ]]; then
        case $_validator in
          valid_yes_no|valid_source|valid_kind|valid_workflow)
            ui_menu "$_label" "$_default" "$_validator" "$_from_env"
            _answer=$REPLY
            ;;
          *)
            ui_screen "$_label"
            if [[ -n "$_default" ]]; then
              ui_paragraph "Default: $_default" "$ui_muted"
            elif [[ $_optional == yes ]]; then
              ui_emit '%s%sOptional. Leave blank to omit.%s\n' "$ui_indent" "$ui_muted" "$ui_reset"
            else
              ui_emit '%s%sRequired field%s\n' "$ui_indent" "$ui_muted" "$ui_reset"
            fi
            if [[ -n $_from_env ]]; then
              ui_paragraph "From environment: $_from_env" "$ui_muted"
            fi
            ui_border top
            ui_row '' "$ui_input"
            ui_border bottom
            ui_emit '\n%sEnter: continue or accept default. Ctrl-C: cancel.\n' "$ui_indent"
            ui_flush
            printf '\033[4A'
            ui_read_field
            _answer=$REPLY
            ;;
        esac
      else
        if [[ -n $_from_env ]]; then printf 'From environment: %s\n' "$_from_env"; fi
        printf '%s' "$_label"
        if [[ -n "$_default" ]]; then printf ' [%s]' "$_default"; fi
        printf ': '
        IFS= read -r _answer <&3 || fail 'Installation cancelled because terminal input ended.'
      fi
      _answer=${_answer#"${_answer%%[![:space:]]*}"}
      _answer=${_answer%"${_answer##*[![:space:]]}"}
      _answer=${_answer:-$_default}
      if [[ -z "$_answer" && $_optional == yes ]]; then
        printf -v "$_dest" '%s' ''
        return
      fi
      if [[ -n "$_answer" && $_answer != *[[:cntrl:]]* ]]; then
        if [[ -z "$_validator" ]] || "$_validator" "$_answer"; then
          printf -v "$_dest" '%s' "$_answer"
          return
        fi
      fi
      if [[ $ui_active == yes ]]; then
        printf '\n  Invalid value. Press Enter to try again.'
        IFS= read -r _answer <&3 || cancel 130
      else
        printf '%s\n' 'Enter a valid value for this field.'
      fi
    done
  }

  # Questions reject control characters; only backslashes and quotes need JSON escaping.
  json_quote() {
    local escaped=${1//\\/\\\\}
    escaped=${escaped//\"/\\\"}
    printf -v REPLY '"%s"' "$escaped"
  }
  secret_questions() {
    local prefix=$4 default_source=existing
    local file_env="${prefix}_FILE" name_env="${prefix}_SECRET_NAME" key_env="${prefix}_SECRET_KEY"
    if [[ -n ${!file_env-} || ( $workflow == basic && -z ${!name_env-} && -z ${!key_env-} ) ]]; then
      default_source=file
    fi
    printf '\n%s\n' "$1"
    printf '%s\n' 'Choose file to create a Secret in a new namespace. Use existing for a provisioned Secret.'
    ask source 'Secret source (existing/file)' "$default_source" no valid_source
    ask secret_name 'Secret name' "$2" no valid_secret "$name_env" default
    ask secret_key 'Secret key' "$3" no valid_key "$key_env" default
    secret_file=''
    if [[ $source == file ]]; then
      ask secret_file 'Absolute path to the private file on this machine' '' no valid_path "$file_env" prefilled
    fi
  }
  ensure_secret() {
    local name=$1 key=$2 file=$3 existing size
    run "${kube[@]}" get secret "$name" --ignore-not-found -o name > "$work_dir/secret-name"
    existing=$(< "$work_dir/secret-name")
    if [[ -n "$existing" ]]; then
      printf 'Reusing Secret %s; its contents are not changed.\n' "$name"
    elif [[ -n "$file" ]]; then
      run "${kube[@]}" create secret generic "$name" "--from-file=$key=$file" > /dev/null
    else
      fail "Required Secret $name is missing; create/sync it before retrying."
    fi
    run "${kube[@]}" get secret "$name" -o "go-template={{len (index .data \"$key\")}}" > "$work_dir/secret-size"
    size=$(< "$work_dir/secret-size")
    [[ $size =~ ^[1-9][0-9]*$ ]] || fail "Secret $name needs a nonempty $key key."
  }

  build_values() {
    json_quote "$zone"; zone_json=$REPLY
    json_quote "$group"; group_json=$REPLY
    json_quote "$backend"; backend_json=$REPLY
    values="{\"inventory\":{\"computeZone\":$zone_json,\"nodeGroup\":$group_json,\"targets\":[$targets]},\"credentials\":{\"existingSecret\":\"$credentials_secret\",\"secretKey\":\"$credentials_key\"},\"backend\":{\"url\":$backend_json,\"$token_field\":{\"existingSecret\":\"$token_secret\",\"secretKey\":\"$token_key\"}}"
    if [[ -n "$pull_secret" ]]; then values+=",\"imagePullSecrets\":[{\"name\":\"$pull_secret\"}]"; fi
    values+='}'
  }

  ui_mode=auto
  ui_glyphs=unicode
  workflow='' environment_defaults=no
  while (( $# )); do
    case $1 in
      --plain) ui_mode=plain ;;
      --tui) ui_mode=tui ;;
      --unicode) ui_glyphs=unicode ;;
      --ascii) ui_glyphs=ascii ;;
      --basic|--advanced)
        if [[ -n $workflow && $workflow != "${1#--}" ]]; then fail 'Choose either --basic or --advanced.'; fi
        workflow=${1#--}
        ;;
      --help|-h)
        printf '%s\n' 'Usage: bash install.sh [--basic | --advanced] [--tui | --plain] [--unicode | --ascii]' \
          'Choose Basic for a single BMC and standard settings, or Advanced to customize the setup.' \
          'Defaults to the Unicode terminal UI; uses plain prompts when the terminal UI is unavailable.' \
          'Use --ascii for basic borders and symbols, or --plain for text prompts.' \
          'Export FLEETINT_INSTALL_ variables to supply prompt defaults. See the installation guide for the list.'
        exit 0
        ;;
      *) fail "Unknown installer option: $1" ;;
    esac
    shift
  done
  if ! { exec 3</dev/tty; } 2>/dev/null; then
    fail 'This installer needs an interactive terminal for its questions.'
  fi
  for tool in helm kubectl mktemp chmod rm sleep; do command -v "$tool" >/dev/null || fail "Install $tool before running this installer."; done
  work_dir=$(mktemp -d "${TMPDIR:-/tmp}/fleetint-install-XXXXXXXX")
  run helm version --short > "$work_dir/helm-version"
  tool_version=$(< "$work_dir/helm-version")
  case $tool_version in
    v3.*) helm_install_args=(--wait) ;;
    # Keep client-side apply and readiness checks consistent across Helm versions.
    v4.*) helm_install_args=(--wait=legacy --server-side=false) ;;
    *) fail 'This installer requires Helm 3 or 4.' ;;
  esac

  ui_section='Get started'
  ui_accent='' ui_bold='' ui_muted='' ui_reset='' ui_selected='' ui_input=''
  ui_columns=${COLUMNS:-80} ui_rows=${LINES:-24}
  if [[ -t 1 ]] && command -v stty >/dev/null; then
    if ! read -r ui_rows ui_columns < <(stty size <&3); then
      printf '%s\n' 'Terminal size unavailable; using an 80-column layout.' >&2
      ui_columns=80 ui_rows=24
    fi
  fi
  [[ $ui_columns =~ ^[0-9]+$ && $ui_columns -ge 1 ]] || ui_columns=80
  [[ $ui_rows =~ ^[0-9]+$ && $ui_rows -ge 1 ]] || ui_rows=24
  if [[ $ui_mode != plain ]] && (( ui_columns < 80 || ui_rows < 24 )); then
    [[ $ui_mode != tui ]] || fail 'The terminal UI needs at least 80 columns and 24 rows. Enlarge the terminal or use --plain.'
    printf '%s\n' 'Terminal smaller than 80x24; using plain text prompts.' >&2
    ui_mode=plain
  fi
  ui_margin=$(((ui_columns - 72) / 2))
  (( ui_margin >= 2 )) || ui_margin=2
  printf -v ui_indent '%*s' "$ui_margin" ''
  ui_text_column=$((ui_margin + 3)) ui_right_column=$((ui_margin + 72))
  if [[ $ui_mode != plain ]] && ! command -v stty >/dev/null; then
    [[ $ui_mode != tui ]] || fail 'The terminal UI needs the standard stty utility. Use --plain for text prompts.'
    printf '%s\n' 'stty unavailable; using plain text prompts.' >&2
    ui_mode=plain
  fi
  if [[ $ui_mode != plain ]]; then
    if [[ -t 1 && ${TERM:-dumb} != dumb ]]; then
      ui_active=yes
      terminal_state=$(stty -g <&3) || fail 'Could not read terminal settings. Use --plain for text prompts.'
      stty -echo -icanon min 1 time 0 <&3 || fail 'Could not prepare terminal input. Use --plain for text prompts.'
      if [[ -z ${NO_COLOR:-} ]]; then
        ui_reset=$'\033[0;37;40m' ui_accent=$'\033[92m' ui_muted=$'\033[37m'
        ui_selected=$'\033[1;30;42m' ui_input=$'\033[37;40m'
        case ${COLORTERM:-}:${TERM:-} in
          truecolor:*|24bit:*|*:xterm-ghostty|*:ghostty)
            ui_reset=$'\033[0;38;2;215;221;229;48;2;16;20;24m'
            ui_accent=$'\033[38;2;118;185;0m'
            ui_muted=$'\033[38;2;138;148;158m'
            ui_selected=$'\033[1;38;2;166;226;46;48;2;35;50;28m'
            ui_input=$'\033[38;2;235;239;243;48;2;24;32;39m'
            ;;
          *) if [[ ${TERM:-} == *256color* ]]; then
            ui_reset=$'\033[0;38;5;252;48;5;233m'
            ui_accent=$'\033[38;5;106m' ui_muted=$'\033[38;5;245m'
            ui_selected=$'\033[1;38;5;155;48;5;22m' ui_input=$'\033[38;5;255;48;5;234m'
          fi ;;
        esac
        ui_bold="$ui_reset"$'\033[1m'
      fi
      ui_top_left='+' ui_top_right='+' ui_bottom_left='+' ui_bottom_right='+' ui_horizontal='-' ui_vertical='|'
      ui_logo_top='[===]' ui_logo_middle='[o:o]' ui_logo_bottom='[===]' ui_pointer='>' ui_complete='+' ui_current='*' ui_pending='o'
      case $ui_glyphs in
        unicode)
          ui_top_left='╭' ui_top_right='╮' ui_bottom_left='╰' ui_bottom_right='╯' ui_horizontal='─' ui_vertical='│'
          ui_logo_top='╭───╮' ui_logo_middle='│▪ ▪│' ui_logo_bottom='╰───╯' ui_pointer='›' ui_complete='✓' ui_current='●' ui_pending='○'
          ;;
      esac
      ui_rule=''
      for ((i=0; i<70; i++)); do ui_rule+=$ui_horizontal; done
      printf '\033[?1049h\033[?2004h%s\033[2J' "$ui_reset"
    elif [[ $ui_mode == tui ]]; then
      fail 'The terminal UI requires an ANSI terminal on stdout. Use --plain for text prompts.'
    else
      printf '%s\n' 'ANSI terminal unavailable; using plain text prompts.' >&2
    fi
  fi

  chart=fleet-intelligence-oob-collector
  repo=https://helm.ngc.nvidia.com/nvidia/fleet-intelligence
  guide=https://docs.nvidia.com/fleet-intel/oob-collector/installation
  printf '%s\n' 'Install the Fleet Intelligence OOB collector on Kubernetes.'
  printf '%s\n' 'Have Helm 3 or 4, kubectl, BMC IPs/MACs, credential profiles, and an enrollment token ready.'
  if [[ -z $workflow ]]; then
    if [[ $ui_active == no ]]; then
      printf '%s\n' 'Basic starts with one BMC and recommended settings. Advanced shows all options.'
    fi
    ask workflow 'Installation workflow (basic/advanced)' basic no valid_workflow
  fi
  current_context=''
  profile_default=''
  if [[ $workflow == basic ]]; then
    ui_section='1 / 4  Setup'
    profile_default=bmc
    if [[ -z ${FLEETINT_INSTALL_KUBE_CONTEXT:-} ]] &&
       run kubectl config current-context > "$work_dir/current-context" 2>/dev/null; then
      current_context=$(< "$work_dir/current-context")
      if [[ $current_context == *[[:cntrl:]]* || $current_context != *[![:space:]]* ]]; then current_context=''; fi
    fi
    if [[ $ui_active == no && -z $current_context && -z ${FLEETINT_INSTALL_KUBE_CONTEXT:-} ]]; then
      printf '%s\n' 'No current Kubernetes context found. Choose a context to continue.'
    fi
  else
    ui_section='1 / 6  Deployment'
  fi
  ask context 'Kubernetes context (kubectl config get-contexts)' "$current_context" no '' FLEETINT_INSTALL_KUBE_CONTEXT prefilled
  ask namespace Namespace fleet-intelligence no valid_label FLEETINT_INSTALL_NAMESPACE default
  ask release 'Helm release name' "$chart" no valid_release FLEETINT_INSTALL_RELEASE default
  latest_version=''
  if [[ -z ${FLEETINT_INSTALL_CHART_VERSION:-} ]]; then
    if [[ $ui_active == yes ]]; then
      ui_screen 'Checking for the latest chart release'
      ui_flush
    else
      printf '%s\n' 'Checking NGC for the latest stable chart release.'
    fi
    if run helm show chart "$chart" --repo "$repo" > "$work_dir/chart.yaml" 2>/dev/null; then
      # Helm selects the latest stable version unless given a version or --devel.
      while IFS= read -r line || [[ -n "$line" ]]; do
        case $line in
          'version: '*)
            candidate=${line#version: }
            if valid_version "$candidate" && [[ ${candidate%%+*} != *-* ]]; then
              latest_version=$candidate
            fi
            break
            ;;
        esac
      done < "$work_dir/chart.yaml"
    fi
    if [[ -z $latest_version && $ui_active == no ]]; then
      printf 'Latest release lookup failed. Enter an exact chart version from the NGC listing in %s\n' "$guide"
    fi
  fi
  ask version 'NGC chart version' "$latest_version" no valid_version FLEETINT_INSTALL_CHART_VERSION prefilled
  ask backend 'Fleet Intelligence HTTPS backend origin' https://data.fleet-intelligence.nvidia.com no valid_backend FLEETINT_INSTALL_BACKEND_URL default
  ask zone 'Default compute zone (optional)' '' yes valid_text FLEETINT_INSTALL_COMPUTE_ZONE default
  ask group 'Default node group (optional)' '' yes valid_text FLEETINT_INSTALL_NODE_GROUP default
  if [[ $workflow == basic ]]; then ui_section='1 / 4  NICo hardware health'
  else ui_section='2 / 6  NICo hardware health'; fi
  if [[ $ui_active == no ]]; then
    printf '\n%s\n' 'This install includes NICo hardware health and its telemetry gateway.'
    printf '%s\n' 'For BMCs without NICo coverage, press Enter to use the bundled setup.'
  fi
  ask existing_nico 'Does NICo already monitor these BMCs? yes/no' no no valid_yes_no
  if [[ $existing_nico == yes ]]; then
    ui_leave
    printf '%s\n' 'To keep your existing NICo deployment, use the manual Helm steps with the bundled NICo and gateway disabled.'
    printf '%s\n' "$guide#keep-an-existing-nico-deployment"
    exit 0
  fi

  if [[ $ui_active == no && $workflow == advanced ]]; then
    printf '\n%s\n' 'For a first install, verify one BMC before adding more.'
  fi
  targets='' seen_macs='' count=0
  target_review=()
  if [[ $workflow == basic ]]; then target_step='2 / 4'; else target_step='3 / 6'; fi
  while :; do
    count=$((count + 1))
    target_env=''
    if (( count == 1 )); then target_env=FLEETINT_INSTALL_BMC_; fi
    ui_section="$target_step  BMC target $count"
    ask kind 'Kind: compute, nvswitch, power_shelf, unknown' unknown no valid_kind "${target_env:+${target_env}KIND}" default
    case $kind in
      compute) device='Compute node' ;;
      nvswitch) device='NVSwitch' ;;
      power_shelf) device='Power shelf' ;;
      unknown) device='Device' ;;
    esac
    ui_section="$target_step  BMC $count ($device)"
    if [[ $ui_active == no ]]; then printf '\nBMC %s (%s)\n' "$count" "$device"; fi
    ask address 'BMC IP address (no scheme or port)' '' no valid_ip "${target_env:+${target_env}IP}" prefilled
    ask mac 'BMC MAC address (unique)' '' no valid_unique_mac "${target_env:+${target_env}MAC}" prefilled
    normalize_mac "$mac"; mac=$REPLY
    seen_macs+="|$mac|"
    ask port 'Redfish HTTPS port' 443 no valid_port "${target_env:+${target_env}PORT}" default
    valid_port "$port"; port=$REPLY
    printf -v target '{"address":"%s","mac":"%s","kind":"%s","port":%s' "$address" "$mac" "$kind" "$port"
    target_review[${#target_review[@]}]="BMC $count ($device): $address"
    target_review[${#target_review[@]}]="Redfish port: $port | MAC: $mac"
    ask name 'Display name (optional)' '' yes valid_text "${target_env:+${target_env}NAME}" default
    ask profile 'Credentials profile (optional with a per-MAC override)' "$profile_default" yes valid_text "${target_env:+${target_env}CREDENTIALS_PROFILE}" default
    ask target_zone 'Compute zone for this BMC (optional)' '' yes valid_text "${target_env:+${target_env}COMPUTE_ZONE}" default
    ask target_group 'Node group for this BMC (optional)' '' yes valid_text "${target_env:+${target_env}NODE_GROUP}" default
    fields=("$name" "$profile" "$target_zone" "$target_group")
    field_names=(name credentialsProfile computeZone nodeGroup)
    field_labels=('Display name' 'Credentials profile' 'Compute zone' 'Node group')
    for ((i=0; i<4; i++)); do
      field=${fields[$i]}
      if [[ -n "$field" ]]; then
        json_quote "$field"
        target+=",\"${field_names[$i]}\":$REPLY"
        target_review[${#target_review[@]}]="${field_labels[$i]}: $field"
      fi
    done
    labels='' label_keys='|' label_count=1
    [[ -z "$name" ]] || label_count=$((label_count + 1))
    [[ -z "${target_zone:-$zone}" ]] || label_count=$((label_count + 1))
    [[ -z "${target_group:-$group}" ]] || label_count=$((label_count + 1))
    while [[ $workflow == advanced ]] && (( label_count < 32 )); do
      ask label_key 'Label key (e.g. rack, blank to finish)' '' yes valid_target_label
      [[ -n "$label_key" ]] || break
      ask label_value "Label value for $label_key (optional)" '' yes valid_text
      json_quote "$label_key"; labels+="${labels:+,}$REPLY:"
      json_quote "$label_value"; labels+=$REPLY
      label_keys+="$label_key|"
      label_count=$((label_count + 1))
      target_review[${#target_review[@]}]="Label $label_key: $label_value"
    done
    if [[ -n "$labels" ]]; then target+=",\"labels\":{$labels}"; fi
    target+='}'
    targets+="${targets:+,}$target"
    [[ $workflow != basic ]] || break
    ask more 'Add another BMC? yes/no' no no valid_yes_no
    [[ $more == yes ]] || break
  done
  if [[ $workflow == basic ]]; then ui_section='3 / 4  BMC credentials'
  else ui_section='4 / 6  BMC credentials'; fi
  secret_questions 'BMC credentials JSON' fleet-intelligence-bmc-credentials credentials.json FLEETINT_INSTALL_BMC_CREDENTIALS
  credentials_secret=$secret_name credentials_key=$secret_key credentials_file=$secret_file
  if [[ $workflow == basic ]]; then ui_section='3 / 4  Enrollment token'
  else ui_section='5 / 6  Enrollment token'; fi
  secret_questions 'Fleet Intelligence enrollment token' "$chart-agent-token" agent-token FLEETINT_INSTALL_ENROLLMENT_TOKEN
  token_secret=$secret_name token_key=$secret_key token_file=$secret_file
  if [[ $credentials_secret == "$token_secret" && ( -n "$credentials_file" || -n "$token_file" ) ]]; then
    fail 'File-backed credentials and token must use different Secret names.'
  fi
  ask pull_secret 'Existing image pull Secret (optional)' '' yes valid_secret FLEETINT_INSTALL_IMAGE_PULL_SECRET default
  for file in "$credentials_file" "$token_file"; do
    [[ -z "$file" || ( -f "$file" && -r "$file" && -s "$file" ) ]] || fail 'A selected private input file is missing, unreadable, or empty.'
  done

  token_field=enrollmentToken
  build_values
  if [[ $workflow == basic ]]; then ui_section='4 / 4  Review and install'
  else ui_section='6 / 6  Review and install'; fi
  if [[ $ui_active == yes ]]; then
    review_lines=()
    review_add "Context: $context"
    review_add "Namespace: $namespace"
    review_add "Release: $release"
    review_add "Chart version: $version"
    review_add "Backend: $backend"
    review_add "Compute zone: ${zone:-(unset)} | Node group: ${group:-(unset)}"
    for line in "${target_review[@]}"; do review_add "$line"; done
    review_add "BMC Secret: $credentials_secret / $credentials_key"
    review_add "BMC input: ${credentials_file:-(existing Secret)}"
    review_add "Token Secret: $token_secret / $token_key"
    review_add "Token input: ${token_file:-(existing Secret)}"
    review_add "Image pull Secret: ${pull_secret:-(chart default)}"
    review_add 'Deploy the collector, NICo health, and gateway with chart defaults.'
    review_add 'Existing Secrets are preserved. New Secrets use your selected files.'
    if [[ $environment_defaults == yes ]]; then review_add 'Defaults loaded from the terminal environment.'; fi
    ui_review
  else
    printf '\nContext: %s\nNamespace: %s\nRelease: %s\nChart version: %s\n%s\n' "$context" "$namespace" "$release" "$version" "$values"
    printf '%s\n' 'Deploy the collector, NICo health, and gateway with chart defaults.'
    if [[ $environment_defaults == yes ]]; then printf '%s\n' 'Defaults loaded from the terminal environment.'; fi
    printf '%s\n' 'For proxies, registry mirrors, or additional collection tuning, use the manual installation guide.'
  fi
  ask confirm 'Install with these settings? yes/no' no no valid_yes_no
  install_ui=$ui_active
  ui_leave
  if [[ $confirm == no ]]; then printf '%s\n' 'Installation cancelled; no cluster changes were made.'; exit 0; fi

  verified_checks=() verification_warnings=no
  kube=(kubectl --context "$context" --namespace "$namespace")
  release_selector="app.kubernetes.io/instance=$release"
  collector_selector="$release_selector,app.kubernetes.io/name=$chart"
  nico_selector="$release_selector,app.kubernetes.io/component=nico-hardware-health"
  gateway_selector="$release_selector,app.kubernetes.io/component=otel-gateway"
  install_status 1/5 'Checking cluster access and chart configuration'
  run "${kube[@]}" get --raw=/version --request-timeout=15s > /dev/null
  run helm show values "$chart" --repo "$repo" --version "$version" > "$work_dir/chart-values.yaml"
  token_field='' in_backend=no
  while IFS= read -r line || [[ -n "$line" ]]; do
    case $line in
      backend:) in_backend=yes ;;
      [![:space:]#]*) in_backend=no ;;
    esac
    [[ $in_backend == yes ]] || continue
    case $line in
      '  enrollmentToken:'*) token_field=enrollmentToken; break ;;
      '  agentToken:'*) token_field=agentToken ;;
    esac
  done < "$work_dir/chart-values.yaml"
  [[ -n "$token_field" ]] || fail 'The selected chart has no supported enrollment token configuration.'
  build_values
  # The private directory prevents path replacement; set permissions through the open handle.
  (set -C; exec 4> "$work_dir/values.json"; chmod 600 /dev/fd/4; printf '%s\n' "$values" >&4)
  chart_args=("$chart" --repo "$repo" --version "$version" --values "$work_dir/values.json" --namespace "$namespace" --kube-context "$context")
  run helm template "$release" "${chart_args[@]}" > /dev/null
  verified_checks+=('Kubernetes API reachable and Helm chart rendered')
  install_status 2/5 'Checking namespace and Secret references'
  run kubectl --context "$context" create namespace "$namespace" --dry-run=client -o yaml > "$work_dir/namespace.yaml"
  run_logged 'Kubernetes namespace apply' kubectl --context "$context" apply -f "$work_dir/namespace.yaml"
  run_logged 'Kubernetes namespace readiness' "${kube[@]}" wait --for=jsonpath='{.status.phase}'=Active "namespace/$namespace" --timeout=30s ||
    fail "Namespace $namespace did not become Active. Check the Kubernetes error above."
  ensure_secret "$credentials_secret" "$credentials_key" "$credentials_file"
  ensure_secret "$token_secret" "$token_key" "$token_file"
  if [[ -n $pull_secret ]]; then
    run "${kube[@]}" get secret "$pull_secret" -o name > /dev/null ||
      fail "Could not read image pull Secret $pull_secret. Check its name and namespace."
  fi
  verified_checks+=('Namespace active and required Secret keys present')
  install_status 3/5 'Installing workloads (Helm waits up to 10 minutes)'
  run_logged 'Helm install or upgrade' helm upgrade --install "$release" "${chart_args[@]}" --create-namespace "${helm_install_args[@]}" --timeout 10m ||
    deployment_failed 'Helm install or upgrade failed.'
  verified_checks+=('Helm release applied')
  install_status 4/5 'Waiting for the collector, NICo, and gateway Deployments'
  verify_rollout 'OOB collector' "$collector_selector,app.kubernetes.io/component!=nico-hardware-health,app.kubernetes.io/component!=otel-gateway"
  collector_deployment=$REPLY
  verified_checks+=('OOB collector Deployment ready')
  verify_rollout 'NICo health' "$nico_selector"
  verified_checks+=('NICo health Deployment ready')
  verify_rollout 'telemetry gateway' "$gateway_selector"
  verified_checks+=('Telemetry gateway Deployment ready')
  run_logged 'Helm release status' helm status "$release" --namespace "$namespace" --kube-context "$context" ||
    deployment_failed 'Could not read Helm release status.'
  run_logged 'Pods for this release' "${kube[@]}" get pods --selector "$release_selector" --request-timeout=10s ||
    deployment_failed 'Could not read Kubernetes pod status.'
  install_status 5/5 'Waiting for a confirmed inventory upload (up to 6 minutes)'
  wait_for_inventory "$collector_deployment" || deployment_failed 'Could not confirm an accepted inventory upload.'
  verified_checks+=('Fleet Intelligence accepted an inventory upload')
  report_workload_warnings collector 'collector' "$collector_selector" collector
  report_workload_warnings nico 'NICo health' "$nico_selector" nico-hardware-health
  report_workload_warnings gateway 'telemetry gateway' "$gateway_selector" otel-gateway
  install_status done 'Kubernetes rollouts and an inventory upload confirmed'
  ui_verification
  show_verified_checks
  printf '\n%s\n' 'Still to verify in Fleet Intelligence' \
    '  Confirm fresh inventory and health telemetry for each BMC MAC.'
  printf 'Health verification and troubleshooting: %s#verify\n' "$guide"
  printf '\n%s\n' 'To retrieve your values for future updates, run:'
  printf '%q ' helm get values "$release" --namespace "$namespace" --kube-context "$context"
  printf '\n'
}

fleetint_install "$@"
