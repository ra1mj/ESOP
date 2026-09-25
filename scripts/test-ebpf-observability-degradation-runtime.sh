#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

CLANG=${CLANG:-clang}
OBJECT=bpf/build/esop_runtime.bpf.o
FIXTURE=target/debug/examples/observability_degradation_qualification
REPORT=build/ebpf_observability_degradation_qualification.json

mkdir -p bpf/build build
python3 - "$REPORT" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
for candidate in (path, pathlib.Path(f"{path}.tmp")):
    candidate.unlink(missing_ok=True)
PY
cp bpf/vmlinux.h bpf/build/vmlinux.h
make -C bpf CLANG="$CLANG"
cargo build -p esop-ebpf-runtime --example observability_degradation_qualification

if [ "$(id -u)" -eq 0 ]; then
    "$FIXTURE" "$OBJECT" "$REPORT"
elif sudo -n true >/dev/null 2>&1; then
    sudo -n "$FIXTURE" "$OBJECT" "$REPORT"
else
    echo "observability degradation eBPF runtime qualification requires root or passwordless sudo; compilation completed but no qualification was claimed" >&2
    exit 1
fi

python3 scripts/validate-ebpf-observability-degradation-qualification.py "$REPORT"
