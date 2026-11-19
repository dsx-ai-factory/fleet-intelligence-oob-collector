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
  trap cleanup EXIT
  trap 'cancel 130' INT
  trap 'cancel 143' TERM
  trap 'ui_leave; printf "%s\n" "Installation stopped after a command failed. Review the diagnostic above, resolve the error, and retry; existing Secrets are preserved." >&2' ERR

  # Waiting on a background child lets the signal traps cancel long Helm operations.
  run() {
    local status=0
    "$@" </dev/null &
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
    if [[ $install_ui == yes ]]; then
      printf '\n  %s[%s]\033[0m %s\n' "$ui_accent" "$1" "$2"
    fi
  }
  ui_screen() {
    local title=$1 hint='One step at a time. Your settings stay on this machine.' step index style marker
    local steps=(Cluster Targets Credentials Access Review)
    step=${ui_section:0:1}
    case $title in
      'Kubernetes context'*) title='Choose your cluster'; hint='Every cluster operation will use this Kubernetes context.' ;;
      Namespace) title='Where should we deploy?'; hint='Use a dedicated namespace to keep the collector together.' ;;
      'Helm release name') title='Name this deployment'; hint='This is the Helm release you will manage and upgrade.' ;;
      'Published NGC chart version'*) title='Choose a chart version'; hint='Enter an exact published version from the NGC chart listing.' ;;
      'Fleet Intelligence HTTPS'*) title='Connect to Fleet Intelligence'; hint='Use the production endpoint or your own HTTPS backend origin.' ;;
      *'BMC IP address'*) title='Add a BMC address'; hint='Enter an IPv4 or IPv6 address, without a scheme or port.' ;;
      *'BMC MAC'*) title='Identify this BMC'; hint='Use the physical BMC MAC, not the host network interface MAC.' ;;
      'Kind: compute'*) title='Choose the hardware type'; hint='This tells Fleet Intelligence what kind of device you collect.' ;;
      'Source: existing or file') title='How should we read the credentials?'; hint='Passwords and tokens stay in your files or Kubernetes Secrets.' ;;
      'Install with these settings?'*) title='Ready to install?'; hint='Install the collector, hardware health, and telemetry gateway.' ;;
      'Review the installation settings') hint='Check the cluster, hardware, and Secret references before installing.' ;;
    esac
    ui_frame=$'\033[H\n'
    ui_emit '%s%s%s%s  %sNVIDIA / FLEET INTELLIGENCE%s\n' "$ui_indent" "$ui_accent" "$ui_logo_top" "$ui_reset" "$ui_bold" "$ui_reset"
    ui_emit '%s%s%s%s  Out-of-band collector\n' "$ui_indent" "$ui_accent" "$ui_logo_middle" "$ui_reset"
    ui_emit '%s%s%s%s  %sHardware visibility, beyond the host.%s\n\n' "$ui_indent" "$ui_accent" "$ui_logo_bottom" "$ui_reset" "$ui_muted" "$ui_reset"
    ui_emit '%s' "$ui_indent"
    for ((index=0; index<5; index++)); do
      style=$ui_muted marker=$ui_pending
      if (( index + 1 < step )); then style=$ui_accent; marker=$ui_complete; fi
      if (( index + 1 == step )); then style=$ui_bold; marker=$ui_current; fi
      ui_emit '%s%s %s%s  ' "$style" "$marker" "${steps[$index]}" "$ui_reset"
    done
    ui_emit '\n\n'
    ui_paragraph "$ui_section" "$ui_muted"
    ui_paragraph "$title" "$ui_bold"
    ui_paragraph "$hint" "$ui_muted"
    ui_emit '\n'
  }
  ui_key() {
    local suffix=''
    REPLY=''
    IFS= read -r -s -n 1 REPLY <&3 || cancel 130
    if [[ $REPLY == $'\033' ]]; then
      if IFS= read -r -s -n 2 -t 1 suffix <&3; then
        case $suffix in '[A'|'OA') REPLY=up ;; '[B'|'OB') REPLY=down ;; *) REPLY=escape ;; esac
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
      IFS= read -r -s -n 1 key <&3 || fail 'Installation cancelled because terminal input ended.'
      if [[ $key == $'\033' ]]; then
        sequence=''
        while (( ${#sequence} < 8 )); do
          part=''
          IFS= read -r -s -n 1 -t 1 part <&3 || break
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
    local label=$1 default=$2 validator=$3 selected=0 index
    local options=() descriptions=() details=()
    case $validator in
      valid_yes_no)
        options=(no yes); descriptions=('No' 'Yes')
        details=('Keep things as they are.' 'Continue with the settings you selected.')
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

  ask() {
    local _dest=$1 _label=$2 _default=${3-} _optional=${4-no} _validator=${5-} _answer
    while :; do
      if [[ $ui_active == yes ]]; then
        case $_validator in
          valid_yes_no|valid_source|valid_kind)
            ui_menu "$_label" "$_default" "$_validator"
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
    printf '\n%s: use an existing Secret (including Vault/OpenBao-managed Secrets),\n' "$1"
    printf '%s\n' 'or create a missing Secret from a private file on this machine.'
    ask source 'Source: existing or file' existing no valid_source
    ask secret_name 'Secret name' "$2" no valid_secret
    ask secret_key 'Secret key' "$3" no valid_key
    secret_file=''
    if [[ $source == file ]]; then
      ask secret_file 'Absolute path to the private file on this machine' '' no valid_path
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
  while (( $# )); do
    case $1 in
      --plain) ui_mode=plain ;;
      --tui) ui_mode=tui ;;
      --unicode) ui_glyphs=unicode ;;
      --ascii) ui_glyphs=ascii ;;
      --help|-h) printf '%s\n' 'Usage: bash install.sh [--tui | --plain] [--unicode | --ascii]' 'Defaults to the Unicode terminal UI; uses plain prompts when the terminal UI is unavailable.' 'Use --ascii for basic borders and symbols, or --plain for text prompts.'; exit 0 ;;
      *) fail "Unknown installer option: $1" ;;
    esac
    shift
  done
  if ! { exec 3</dev/tty; } 2>/dev/null; then
    fail 'This installer needs an interactive terminal for its questions.'
  fi
  for tool in helm kubectl mktemp chmod rm; do command -v "$tool" >/dev/null || fail "Install $tool before running this installer."; done
  work_dir=$(mktemp -d "${TMPDIR:-/tmp}/fleetint-install-XXXXXXXX")
  run helm version --short > "$work_dir/helm-version"
  tool_version=$(< "$work_dir/helm-version")
  case $tool_version in
    v3.*) helm_install_args=(--wait) ;;
    # Keep client-side apply and readiness checks consistent across Helm versions.
    v4.*) helm_install_args=(--wait=legacy --server-side=false) ;;
    *) fail 'This installer requires Helm 3 or 4.' ;;
  esac

  ui_section='1 / 5  Deployment'
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
  printf '%s\n' 'Install the Fleet Intelligence OOB collector on Kubernetes.'
  printf '%s\n' 'Have Helm 3 or 4, kubectl, BMC IPs/MACs, credential profiles, and an enrollment token ready.'
  ask context 'Kubernetes context (kubectl config get-contexts)'
  ask namespace Namespace fleet-intelligence no valid_label
  ask release 'Helm release name' "$chart" no valid_release
  ask version 'Published NGC chart version (exact version, e.g. 1.2.3)' '' no valid_version
  ask backend 'Fleet Intelligence HTTPS backend origin' https://data.fleet-intelligence.nvidia.com no valid_backend
  ask zone 'Default compute zone (blank to omit)' '' yes valid_text
  ask group 'Default node group (blank to omit)' '' yes valid_text
  targets='' seen_macs='' count=0
  target_review=()
  while :; do
    count=$((count + 1))
    ui_section="2 / 5  BMC target $count"
    ask kind 'Kind: compute, nvswitch, power_shelf, unknown' unknown no valid_kind
    case $kind in
      compute) device='Compute node' ;;
      nvswitch) device='NVSwitch' ;;
      power_shelf) device='Power shelf' ;;
      unknown) device='Device' ;;
    esac
    ui_section="2 / 5  $device $count"
    ask address "$device BMC IP address (no scheme or port)" '' no valid_ip
    ask mac "$device BMC MAC (unique)" '' no valid_unique_mac
    normalize_mac "$mac"; mac=$REPLY
    seen_macs+="|$mac|"
    ask port "$device Redfish HTTPS port" 443 no valid_port
    valid_port "$port"; port=$REPLY
    printf -v target '{"address":"%s","mac":"%s","kind":"%s","port":%s' "$address" "$mac" "$kind" "$port"
    target_review[${#target_review[@]}]="$device $count: $address:$port | $mac"
    ask name "$device display name (optional)" '' yes valid_text
    ask profile "$device credentials profile (blank uses a per-MAC override)" '' yes valid_text
    ask target_zone "$device compute zone override (blank inherits default)" '' yes valid_text
    ask target_group "$device node group override (blank inherits default)" '' yes valid_text
    fields=("$name" "$profile" "$target_zone" "$target_group")
    field_names=(name credentialsProfile computeZone nodeGroup)
    for ((i=0; i<4; i++)); do
      field=${fields[$i]}
      if [[ -n "$field" ]]; then
        json_quote "$field"
        target+=",\"${field_names[$i]}\":$REPLY"
        target_review[${#target_review[@]}]="${field_names[$i]}: $field"
      fi
    done
    labels='' label_keys='|' label_count=1
    [[ -z "$name" ]] || label_count=$((label_count + 1))
    [[ -z "${target_zone:-$zone}" ]] || label_count=$((label_count + 1))
    [[ -z "${target_group:-$group}" ]] || label_count=$((label_count + 1))
    while (( label_count < 32 )); do
      ask label_key "$device label key (e.g. rack; blank to finish)" '' yes valid_target_label
      [[ -n "$label_key" ]] || break
      ask label_value "$device label value for $label_key (optional)" '' yes valid_text
      json_quote "$label_key"; labels+="${labels:+,}$REPLY:"
      json_quote "$label_value"; labels+=$REPLY
      label_keys+="$label_key|"
      label_count=$((label_count + 1))
      target_review[${#target_review[@]}]="Label $label_key: $label_value"
    done
    if [[ -n "$labels" ]]; then target+=",\"labels\":{$labels}"; fi
    target+='}'
    targets+="${targets:+,}$target"
    ask more 'Add another BMC? yes/no' no no valid_yes_no
    [[ $more == yes ]] || break
  done
  ui_section='3 / 5  BMC credentials'
  secret_questions 'BMC credentials JSON' fleet-intelligence-bmc-credentials credentials.json
  credentials_secret=$secret_name credentials_key=$secret_key credentials_file=$secret_file
  ui_section='4 / 5  Enrollment token'
  secret_questions 'Fleet Intelligence enrollment token' "$chart-agent-token" agent-token
  token_secret=$secret_name token_key=$secret_key token_file=$secret_file
  if [[ $credentials_secret == "$token_secret" && ( -n "$credentials_file" || -n "$token_file" ) ]]; then
    fail 'File-backed credentials and token must use different Secret names.'
  fi
  ask pull_secret 'Existing image pull Secret (optional)' '' yes valid_secret
  for file in "$credentials_file" "$token_file"; do
    [[ -z "$file" || ( -f "$file" && -r "$file" && -s "$file" ) ]] || fail 'A selected private input file is missing, unreadable, or empty.'
  done

  token_field=enrollmentToken
  build_values
  ui_section='5 / 5  Review and install'
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
    review_add 'All three workloads and chart collection/TLS defaults are preserved.'
    review_add 'Existing Secrets are preserved. New Secrets use your selected files.'
    ui_review
  else
    printf '\nContext: %s\nNamespace: %s\nRelease: %s\nChart version: %s\n%s\n' "$context" "$namespace" "$release" "$version" "$values"
    printf '%s\n' 'All three chart workloads and their collection/TLS defaults are preserved.'
    printf '%s\n' 'For proxies, registry mirrors, or collection tuning, use the manual installation guide.'
  fi
  ask confirm 'Install with these settings? yes/no' no no valid_yes_no
  install_ui=$ui_active
  ui_leave
  if [[ $confirm == no ]]; then printf '%s\n' 'Installation cancelled; no cluster changes were made.'; exit 0; fi

  kube=(kubectl --context "$context" --namespace "$namespace")
  install_status 1/4 'Checking cluster access and chart configuration'
  run "${kube[@]}" get --raw=/version > /dev/null
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
  install_status 2/4 'Preparing the namespace and Secret references'
  run kubectl --context "$context" create namespace "$namespace" --dry-run=client -o yaml > "$work_dir/namespace.yaml"
  run kubectl --context "$context" apply -f "$work_dir/namespace.yaml"
  ensure_secret "$credentials_secret" "$credentials_key" "$credentials_file"
  ensure_secret "$token_secret" "$token_key" "$token_file"
  install_status 3/4 'Installing workloads (Helm waits up to 10 minutes)'
  run helm upgrade --install "$release" "${chart_args[@]}" --create-namespace "${helm_install_args[@]}" --timeout 10m
  install_status 4/4 'Checking the Helm release and pods'
  run helm status "$release" --namespace "$namespace" --kube-context "$context"
  run "${kube[@]}" get pods -l "app.kubernetes.io/instance=$release"
  install_status done 'Helm installation complete'
  printf '%s\n' 'Check collector uploads, NICo init-container completion, and gateway enrollment/export logs as described in the installation guide.'
  printf '%s\n' 'To retrieve your values for future updates, run:'
  printf '%q ' helm get values "$release" --namespace "$namespace" --kube-context "$context"
  printf '\n'
}

fleetint_install "$@"
