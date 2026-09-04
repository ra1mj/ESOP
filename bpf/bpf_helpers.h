#ifndef ESOP_BPF_HELPERS_H
#define ESOP_BPF_HELPERS_H

/*
 * Minimal helper declarations for the portable ESOP tracepoint bundle.
 * The build intentionally avoids a distro-specific libbpf header path.
 */
#ifndef __VMLINUX_H__
typedef unsigned char __u8;
typedef unsigned short __u16;
typedef unsigned int __u32;
typedef unsigned long long __u64;
typedef signed int __s32;
typedef signed long long __s64;
#endif

#define SEC(NAME) __attribute__((section(NAME), used))
#define __always_inline inline __attribute__((always_inline))
#define __uint(name, value) int (*name)[value]
#define __type(name, value) typeof(value) *name
#ifdef __clang__
#define BPF_CORE_READ(src, field) __builtin_preserve_access_index((src)->field)
#else
#define BPF_CORE_READ(src, field) ((src)->field)
#endif

#define BPF_ANY 0
#define BPF_MAP_TYPE_HASH 1
#define BPF_MAP_TYPE_ARRAY 2
#define BPF_MAP_TYPE_PERCPU_ARRAY 6
#define BPF_MAP_TYPE_RINGBUF 27

static void *(*bpf_map_lookup_elem)(void *map, const void *key) = (void *)1;
static long (*bpf_map_update_elem)(void *map, const void *key, const void *value,
                                   __u64 flags) = (void *)2;
static long (*bpf_map_delete_elem)(void *map, const void *key) = (void *)3;
static __u64 (*bpf_ktime_get_ns)(void) = (void *)5;
static __u32 (*bpf_get_smp_processor_id)(void) = (void *)8;
static __u64 (*bpf_get_current_pid_tgid)(void) = (void *)14;
static long (*bpf_ringbuf_output)(void *ringbuf, void *data, __u64 size,
                                  __u64 flags) = (void *)130;

#endif
