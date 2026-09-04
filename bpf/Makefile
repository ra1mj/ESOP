CLANG ?= clang
BPFTOOL ?= bpftool
OUT ?= build
ARCH ?= x86
VMLINUX ?= /sys/kernel/btf/vmlinux

.PHONY: all clean

all: $(OUT)/esop_runtime.bpf.o

$(OUT):
	mkdir -p $(OUT)

$(OUT)/vmlinux.h: | $(OUT)
	$(BPFTOOL) btf dump file $(VMLINUX) format c > $@

$(OUT)/esop_runtime.bpf.o: esop_runtime.bpf.c bpf_helpers.h $(OUT)/vmlinux.h | $(OUT)
	$(CLANG) -O2 -g -target bpf -D__TARGET_ARCH_$(ARCH) -I$(OUT) -I. -c $< -o $@

clean:
	rm -rf $(OUT)
