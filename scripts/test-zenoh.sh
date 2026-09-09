#!/usr/bin/env bash
set -euo pipefail

zenohd_bin="${ZENOHD:-$(command -v zenohd || true)}"
if [[ -z "${zenohd_bin}" ]]; then
    printf '%s\n' "zenohd is required; install zenohd 1.10.1 or set ZENOHD" >&2
    exit 2
fi

log_file="$(mktemp)"
zenohd_pid=""
cleanup() {
    if [[ -n "${zenohd_pid}" ]]; then
        kill "${zenohd_pid}" 2>/dev/null || true
        wait "${zenohd_pid}" 2>/dev/null || true
    fi
    rm -f "${log_file}"
}
trap cleanup EXIT

"${zenohd_bin}" --listen tcp/127.0.0.1:17447 >"${log_file}" 2>&1 &
zenohd_pid=$!
sleep 1

"${CARGO:-cargo}" test -p esop-zenoh-gateway --features zenoh --test zenoh_live -- --ignored --nocapture
