#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

CLANG=${CLANG:-clang}
OBJECT=bpf/build/esop_runtime.bpf.o
FIXTURE=target/debug/examples/process_exit_qualification
REPORT=build/ebpf_process_exit_qualification.json

mkdir -p bpf/build build
rm -f "$REPORT" "$REPORT.tmp"
cp bpf/vmlinux.h bpf/build/vmlinux.h
make -C bpf CLANG="$CLANG"
cargo build -p esop-ebpf-runtime --example process_exit_qualification

if [ "$(id -u)" -eq 0 ]; then
    "$FIXTURE" "$OBJECT" "$REPORT"
elif sudo -n true >/dev/null 2>&1; then
    sudo -n "$FIXTURE" "$OBJECT" "$REPORT"
else
    echo "process-exit eBPF runtime qualification requires root or passwordless sudo; compilation completed but no qualification was claimed" >&2
    exit 1
fi

python3 scripts/validate-ebpf-process-exit-qualification.py "$REPORT"
