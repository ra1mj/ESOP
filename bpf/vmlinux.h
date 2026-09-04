#ifndef __VMLINUX_H__
#define __VMLINUX_H__

/*
 * Minimal fallback for syntax checks on hosts without a generated vmlinux.h.
 * `make` generates a complete copy from /sys/kernel/btf/vmlinux and places it
 * before this file on the include path.
 */
#if defined(__clang__) && !defined(BPF_NO_PRESERVE_ACCESS_INDEX)
#pragma clang attribute push (__attribute__((preserve_access_index)), apply_to = record)
#endif

typedef unsigned char __u8;
typedef unsigned short __u16;
typedef unsigned int __u32;
typedef unsigned long long __u64;
typedef signed int __s32;
typedef int pid_t;

struct trace_entry {
    unsigned short type;
    unsigned char flags;
    unsigned char preempt_count;
    int pid;
};

struct trace_event_raw_sched_wakeup_template {
    struct trace_entry ent;
    char comm[16];
    pid_t pid;
    int prio;
    int target_cpu;
    char __data[0];
};

struct trace_event_raw_sched_switch {
    struct trace_entry ent;
    char prev_comm[16];
    pid_t prev_pid;
    int prev_prio;
    long prev_state;
    char next_comm[16];
    pid_t next_pid;
    int next_prio;
    char __data[0];
};

#if defined(__clang__) && !defined(BPF_NO_PRESERVE_ACCESS_INDEX)
#pragma clang attribute pop
#endif

#endif
