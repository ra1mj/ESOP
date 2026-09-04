#include "vmlinux.h"
#include "bpf_helpers.h"

struct esop_context {
    __u64 boot_id;
    __u64 agent_epoch;
    __u64 cycle_seq;
    __u64 transition_seq;
    __u32 tracked_pid;
    __u32 reserved;
    __u64 scheduler_latency_threshold_ns;
    __u64 network_drop_threshold;
};

struct esop_stats {
    __u64 emitted_events;
    __u64 lost_events;
    __u64 wakeups;
    __u64 scheduler_stalls;
    __u64 page_faults;
    __u64 process_exits;
    __u64 oom_events;
};

struct esop_runtime_evidence {
    __u64 evidence_id;
    __u64 boot_id;
    __u64 agent_epoch;
    __u64 timestamp_ns;
    __u64 cycle_seq;
    __u64 transition_seq;
    __u32 pid;
    __u32 tid;
    __u16 cpu;
    __u16 irq;
    __u32 netdev_ifindex;
    __u64 observed_value;
    __u64 threshold;
    __u64 duration_ns;
    __u32 count;
    __u8 domain;
    __u8 kind;
    __u8 severity;
    __u8 reserved;
};

_Static_assert(sizeof(struct esop_context) == 56, "context ABI changed");
_Static_assert(sizeof(struct esop_stats) == 56, "stats ABI changed");
_Static_assert(sizeof(struct esop_runtime_evidence) == 96, "evidence ABI changed");

struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 1 << 22);
} ESOP_EVENTS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct esop_context);
} ESOP_CONTEXT SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct esop_stats);
} ESOP_STATS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 4096);
    __type(key, __u32);
    __type(value, __u64);
} ESOP_WAKEUPS SEC(".maps");

static __always_inline struct esop_context *esop_context(void)
{
    __u32 key = 0;
    return bpf_map_lookup_elem(&ESOP_CONTEXT, &key);
}

static __always_inline struct esop_stats *esop_stats(void)
{
    __u32 key = 0;
    return bpf_map_lookup_elem(&ESOP_STATS, &key);
}

static __always_inline __u32 esop_tgid(void)
{
    return (__u32)(bpf_get_current_pid_tgid() >> 32);
}

static __always_inline __u32 esop_tid(void)
{
    return (__u32)bpf_get_current_pid_tgid();
}

static __always_inline int esop_tracks(__u32 pid, const struct esop_context *context)
{
    return context && (context->tracked_pid == 0 || context->tracked_pid == pid);
}

static __always_inline void esop_emit(__u8 domain, __u8 kind, __u8 severity,
                                      __u64 observed_value, __u64 threshold,
                                      __u64 duration_ns, __u32 count)
{
    struct esop_context *context = esop_context();
    struct esop_stats *stats = esop_stats();
    if (!context) {
        return;
    }

    __u64 now = bpf_ktime_get_ns();
    __u64 timestamp = now;
    struct esop_runtime_evidence event = {};

    /* The timestamp is monotonic and unique enough for the bounded evidence ID. */
    event.evidence_id = timestamp;
    event.boot_id = context->boot_id;
    event.agent_epoch = context->agent_epoch;
    event.timestamp_ns = timestamp;
    event.cycle_seq = context->cycle_seq;
    event.transition_seq = context->transition_seq;
    event.pid = esop_tgid();
    event.tid = esop_tid();
    event.cpu = (__u16)bpf_get_smp_processor_id();
    event.irq = 0;
    event.netdev_ifindex = 0;
    event.observed_value = observed_value;
    event.threshold = threshold;
    event.duration_ns = duration_ns;
    event.count = count;
    event.domain = domain;
    event.kind = kind;
    event.severity = severity;
    event.reserved = 0;
    if (bpf_ringbuf_output(&ESOP_EVENTS, &event, sizeof(event), 0) < 0) {
        if (stats) {
            stats->lost_events++;
        }
    } else if (stats) {
        stats->emitted_events++;
    }
}

SEC("tracepoint/sched/sched_wakeup")
int esop_sched_wakeup(struct trace_event_raw_sched_wakeup_template *event)
{
    struct esop_context *context = esop_context();
    __u32 pid = (__u32)BPF_CORE_READ(event, pid);
    if (!esop_tracks(pid, context)) {
        return 0;
    }
    __u64 now = bpf_ktime_get_ns();
    bpf_map_update_elem(&ESOP_WAKEUPS, &pid, &now, BPF_ANY);
    struct esop_stats *stats = esop_stats();
    if (stats) {
        stats->wakeups++;
    }
    return 0;
}

SEC("tracepoint/sched/sched_switch")
int esop_sched_switch(struct trace_event_raw_sched_switch *event)
{
    struct esop_context *context = esop_context();
    __u32 pid = (__u32)BPF_CORE_READ(event, next_pid);
    if (!esop_tracks(pid, context)) {
        return 0;
    }
    __u64 *start = bpf_map_lookup_elem(&ESOP_WAKEUPS, &pid);
    if (!start) {
        return 0;
    }
    __u64 now = bpf_ktime_get_ns();
    __u64 latency = now - *start;
    bpf_map_delete_elem(&ESOP_WAKEUPS, &pid);
    if (context && latency > context->scheduler_latency_threshold_ns) {
        esop_emit(0, 0, 2, latency, context->scheduler_latency_threshold_ns, latency, 1);
        struct esop_stats *stats = esop_stats();
        if (stats) {
            stats->scheduler_stalls++;
        }
    }
    return 0;
}

SEC("tracepoint/sched/sched_process_exit")
int esop_process_exit(void *ctx)
{
    (void)ctx;
    struct esop_context *context = esop_context();
    if (esop_tracks(esop_tgid(), context)) {
        esop_emit(4, 5, 3, 1, 1, 0, 1);
        struct esop_stats *stats = esop_stats();
        if (stats) {
            stats->process_exits++;
        }
    }
    return 0;
}

SEC("tracepoint/exceptions/page_fault_user")
int esop_page_fault_user(void *ctx)
{
    (void)ctx;
    struct esop_context *context = esop_context();
    if (esop_tracks(esop_tgid(), context)) {
        esop_emit(3, 3, 1, 1, 1, 0, 1);
        struct esop_stats *stats = esop_stats();
        if (stats) {
            stats->page_faults++;
        }
    }
    return 0;
}

SEC("tracepoint/oom/mark_victim")
int esop_oom_kill(void *ctx)
{
    (void)ctx;
    struct esop_context *context = esop_context();
    if (esop_tracks(esop_tgid(), context)) {
        esop_emit(3, 4, 3, 1, 1, 0, 1);
        struct esop_stats *stats = esop_stats();
        if (stats) {
            stats->oom_events++;
        }
    }
    return 0;
}

SEC("tracepoint/skb/kfree_skb")
int esop_network_drop(void *ctx)
{
    (void)ctx;
    struct esop_context *context = esop_context();
    if (esop_tracks(esop_tgid(), context)) {
        esop_emit(2, 2, 2, 1, context ? context->network_drop_threshold : 1, 0, 1);
    }
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
