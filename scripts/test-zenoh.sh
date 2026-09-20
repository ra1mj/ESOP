#!/usr/bin/env bash
set -euo pipefail

zenohd_bin="${ZENOHD:-$(command -v zenohd || true)}"
if [[ -z "${zenohd_bin}" ]]; then
    printf '%s\n' "zenohd is required; install zenohd 1.10.1 or set ZENOHD" >&2
    exit 2
fi

export ZENOHD="${zenohd_bin}"
# Tests own isolated router processes and reap them even on panic.
exec "${CARGO:-cargo}" test -p esop-zenoh-gateway --features zenoh --test zenoh_live -- --ignored --nocapture
