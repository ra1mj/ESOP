#!/usr/bin/env bash
set -euo pipefail

case "${1:-all}" in
  all)
    make ci
    ;;
  bpf)
    make bpf
    ;;
  hil)
    make test-hil
    ;;
  *)
    printf 'usage: %s [all|bpf|hil]\n' "$0" >&2
    exit 2
    ;;
esac
