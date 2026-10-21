#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
method=PUT
conditional=false
while (( $# )); do
    case "$1" in
        --max-redirs) [[ $2 == 0 ]]; shift ;;
        --location|-L) exit 1 ;;
        --netrc-file) credentials=$2; shift ;;
        --head) method=HEAD ;;
        --dump-header) headers=$2; shift ;;
        --upload-file) source=$2; shift ;;
        --header) [[ $2 != 'If-None-Match: *' ]] || conditional=true; shift ;;
    esac
    [[ $1 != *test-repository-token* ]]
    destination=$1
    shift
done
[[ $(stat -c %a "$credentials") == 600 ]]
name=${destination##*/}
name=${name%%;*}
target="$UPLOAD_STORE/$name"
printf '%s %s\n' "$method" "$name" >> "$HTTP_LOG"
if [[ ${REDIRECT:-} == 1 ]]; then printf 302; exit 0; fi
if [[ $method == HEAD ]]; then
    : > "$headers"
    if [[ -f $target ]]; then
        printf 'X-Checksum-Sha256: %s\r\n' "$(sha256sum "$target" | cut -d' ' -f1)" > "$headers"
        printf 200
    else
        printf 404
    fi
else
    "$conditional"
    [[ ! -e $target ]]
    cp "$source" "$target"
    printf 201
fi
