SHELL := /bin/sh

CARGO ?= cargo
RUSTUP ?= rustup
RUST_TARGET ?= aarch64-unknown-none

.PHONY: test test-hil test-ipc test-zenoh test-ebpf-gateway-runtime test-ebpf-raw-port-runtime test-ebpf-process-exit-runtime test-ebpf-oom-runtime test-ebpf-scheduler-migration-runtime test-ebpf-scheduler-runqueue-runtime test-ebpf-softirq-runtime test-ebpf-page-fault-runtime test-ebpf-network-drop-runtime test-ebpf-observability-degradation-runtime check fmt-check lint release no-std bpf-syntax bpf capability-manifest proto-schema build-report performance-report ebpf-gateway-report ebpf-raw-port-report ebpf-process-exit-report ebpf-oom-report ebpf-scheduler-migration-report ebpf-scheduler-runqueue-report ebpf-softirq-report ebpf-page-fault-report ebpf-network-drop-report ebpf-observability-degradation-report r2-qualification zenoh-check setup-rust ci

test:
	$(CARGO) test --workspace --all-features

test-hil:
	$(CARGO) test -p esop-ethercat-linux-port --all-features

test-ipc:
	$(CARGO) test -p esop-ipc --all-features

test-zenoh:
	./scripts/test-zenoh.sh

test-ebpf-gateway-runtime:
	./scripts/test-ebpf-gateway-runtime.sh

test-ebpf-raw-port-runtime:
	./scripts/test-ebpf-raw-port-runtime.sh

test-ebpf-process-exit-runtime:
	./scripts/test-ebpf-process-exit-runtime.sh

test-ebpf-oom-runtime:
	./scripts/test-ebpf-oom-runtime.sh

test-ebpf-scheduler-migration-runtime:
	./scripts/test-ebpf-scheduler-migration-runtime.sh

test-ebpf-scheduler-runqueue-runtime:
	./scripts/test-ebpf-scheduler-runqueue-runtime.sh

test-ebpf-softirq-runtime:
	./scripts/test-ebpf-softirq-runtime.sh

test-ebpf-page-fault-runtime:
	./scripts/test-ebpf-page-fault-runtime.sh

test-ebpf-network-drop-runtime:
	./scripts/test-ebpf-network-drop-runtime.sh

test-ebpf-observability-degradation-runtime:
	./scripts/test-ebpf-observability-degradation-runtime.sh

check:
	$(CARGO) check --workspace --all-features

fmt-check:
	$(CARGO) fmt --all -- --check
	git diff --check

lint:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

release:
	$(CARGO) build --workspace --release

no-std:
	@if ! $(RUSTUP) target list --installed | awk '{print $$1}' | grep -qx '$(RUST_TARGET)'; then \
		echo "missing Rust target $(RUST_TARGET); run 'make setup-rust' first" >&2; \
		exit 1; \
	fi
	$(CARGO) check -p esop-ethercat-core --target $(RUST_TARGET)

# GCC checks the C syntax and ABI declarations without requiring a BPF target.
bpf-syntax:
	gcc -std=gnu11 -Wall -Wextra -Werror -fsyntax-only -Ibpf bpf/esop_runtime.bpf.c

# Requires clang, bpftool, kernel BTF, and a Linux kernel with BPF support.
bpf:
	$(MAKE) -C bpf

capability-manifest:
	python3 scripts/validate-capability-manifest.py

proto-schema:
	python3 scripts/validate-proto-schema.py

build-report:
	python3 scripts/generate-robot-build-report.py --output build/robot_build_report.json
	python3 scripts/validate-robot-build-report.py build/robot_build_report.json

performance-report:
	python3 scripts/generate-performance-report.py --output build/performance_report.json
	python3 scripts/validate-performance-report.py build/performance_report.json
	python3 -m unittest discover -s scripts/tests -p 'test_performance_report.py'

ebpf-gateway-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_gateway_qualification.py'

ebpf-raw-port-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_raw_port_qualification.py'

ebpf-process-exit-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_process_exit_qualification.py'

ebpf-oom-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_oom_qualification.py'

ebpf-scheduler-migration-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_scheduler_migration_qualification.py'

ebpf-scheduler-runqueue-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_scheduler_runqueue_qualification.py'

ebpf-softirq-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_softirq_qualification.py'

ebpf-page-fault-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_page_fault_qualification.py'

ebpf-network-drop-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_network_drop_qualification.py'

ebpf-observability-degradation-report:
	python3 -m unittest discover -s scripts/tests -p 'test_ebpf_observability_degradation_qualification.py'

r2-qualification:
	python3 scripts/validate-r2-qualification.py --expected-commit $$(git rev-parse HEAD) --output build/r2_qualification_report.json
	python3 -m unittest discover -s scripts/tests -p 'test_r2_qualification.py'

zenoh-check:
	$(CARGO) check -p esop-zenoh-gateway --features zenoh

setup-rust:
	$(RUSTUP) target add $(RUST_TARGET)

ci: fmt-check check test lint release no-std bpf-syntax capability-manifest proto-schema build-report performance-report ebpf-gateway-report ebpf-raw-port-report ebpf-process-exit-report ebpf-oom-report ebpf-scheduler-migration-report ebpf-scheduler-runqueue-report ebpf-softirq-report ebpf-page-fault-report ebpf-network-drop-report ebpf-observability-degradation-report r2-qualification zenoh-check
