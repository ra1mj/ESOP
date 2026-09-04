SHELL := /bin/sh

CARGO ?= cargo
RUSTUP ?= rustup
RUST_TARGET ?= aarch64-unknown-none

.PHONY: test test-hil check fmt-check lint release no-std bpf-syntax bpf setup-rust ci

test:
	$(CARGO) test --workspace --all-features

test-hil:
	$(CARGO) test -p esop-ethercat-linux-port --all-features

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

setup-rust:
	$(RUSTUP) target add $(RUST_TARGET)

ci: fmt-check check test lint release no-std bpf-syntax
