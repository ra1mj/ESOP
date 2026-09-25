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

struct trace_event_raw_sched_migrate_task {
    struct trace_entry ent;
    char comm[16];
    pid_t pid;
    int prio;
    int orig_cpu;
    int dest_cpu;
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

struct trace_event_raw_exceptions {
    struct trace_entry ent;
    unsigned long address;
    unsigned long ip;
    unsigned long error_code;
    char __data[0];
};

struct trace_event_raw_mark_victim {
    struct trace_entry ent;
    int pid;
    char __data[0];
};

struct trace_event_raw_irq_handler_entry {
    struct trace_entry ent;
    int irq;
    __u32 __data_loc_name;
    char __data[0];
};

struct trace_event_raw_irq_handler_exit {
    struct trace_entry ent;
    int irq;
    int ret;
    char __data[0];
};

struct trace_event_raw_softirq {
    struct trace_entry ent;
    unsigned int vec;
    char __data[0];
};

struct trace_event_raw_cpu_frequency_limits {
    struct trace_entry ent;
    __u32 min_freq;
    __u32 max_freq;
    __u32 cpu_id;
    char __data[0];
};

struct net_device {
    int ifindex;
};

struct sk_buff {
    struct sk_buff *next;
    struct sk_buff *prev;
    struct net_device *dev;
    int skb_iif;
};

struct trace_event_raw_kfree_skb {
    struct trace_entry ent;
    void *skbaddr;
    void *location;
    const void *rx_sk;
    unsigned short protocol;
    int reason;
    char __data[0];
};

#if defined(__clang__) && !defined(BPF_NO_PRESERVE_ACCESS_INDEX)
#pragma clang attribute pop
#endif

#endif
