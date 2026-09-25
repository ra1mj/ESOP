#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

CLANG=${CLANG:-clang}
OBJECT=bpf/build/esop_runtime.bpf.o
FIXTURE=target/debug/examples/network_drop_qualification
REPORT=build/ebpf_network_drop_qualification.json

if ! command -v ip >/dev/null 2>&1; then
    echo "network-drop eBPF runtime qualification requires iproute2" >&2
    exit 1
fi

mkdir -p bpf/build build
rm -f "$REPORT" "$REPORT.tmp"
cp bpf/vmlinux.h bpf/build/vmlinux.h
make -C bpf CLANG="$CLANG"
cargo build -p esop-ebpf-runtime --example network_drop_qualification

if [ "$(id -u)" -eq 0 ]; then
    "$FIXTURE" "$OBJECT" "$REPORT"
elif sudo -n true >/dev/null 2>&1; then
    sudo -n "$FIXTURE" "$OBJECT" "$REPORT"
else
    echo "network-drop eBPF runtime qualification requires root or passwordless sudo; compilation completed but no qualification was claimed" >&2
    exit 1
fi

python3 scripts/validate-ebpf-network-drop-qualification.py "$REPORT"
