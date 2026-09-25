#include "vmlinux.h"
#include "bpf_helpers.h"

struct esop_context {
    __u64 boot_id;
    __u64 agent_epoch;
    __u64 cycle_seq;
    __u64 transition_seq;
    __u32 tracked_pid;
    __u32 network_ifindex;
    __u64 scheduler_latency_threshold_ns;
    __u64 page_fault_threshold;
    __u64 page_fault_window_ns;
    __u64 network_drop_threshold;
    __u64 network_drop_window_ns;
    __u64 irq_duration_threshold_ns;
    __u64 softirq_duration_threshold_ns;
    __u16 network_protocol;
    __u16 reserved16;
    __u32 reserved32;
    __u32 cpu_frequency_floor_khz;
    __u32 cpu_frequency_policy_cpu;
    __u32 cpu_frequency_policy_epoch;
    __u32 reserved_cpu_frequency;
    __u32 scheduler_tid;
    __u32 scheduler_migration_epoch;
    __u64 scheduler_migration_threshold;
    __u64 scheduler_migration_window_ns;
    __u64 gateway_stall_threshold_ns;
    __u32 gateway_probe_epoch;
    __u32 reserved_gateway;
    __u64 raw_port_stall_threshold_ns;
    __u32 raw_port_probe_epoch;
    __u32 reserved_raw_port;
};

struct esop_stats {
    __u64 emitted_events;
    __u64 lost_events;
    __u64 wakeups;
    __u64 scheduler_stalls;
    __u64 page_faults;
    __u64 process_exits;
    __u64 oom_events;
    __u64 irq_samples;
    __u64 irq_overruns;
    __u64 softirq_samples;
    __u64 softirq_overruns;
    __u64 network_drops;
    __u64 network_unattributed;
    __u64 network_threshold_events;
    __u64 page_fault_threshold_events;
    __u64 thread_exits_ignored;
    __u64 cpu_frequency_updates;
    __u64 cpu_frequency_throttle_events;
    __u64 cpu_frequency_recoveries;
    __u64 cpu_frequency_suppressed;
    __u64 scheduler_migrations;
    __u64 scheduler_migration_threshold_events;
    __u64 gateway_probe_begins;
    __u64 gateway_probe_completions;
    __u64 gateway_stalls;
    __u64 gateway_probe_mismatches;
    __u64 raw_port_probe_begins;
    __u64 raw_port_probe_completions;
    __u64 raw_port_stalls;
    __u64 raw_port_probe_mismatches;
};

struct esop_interrupt_key {
    __u32 cpu;
    __u32 vector;
};

struct esop_network_drop_key {
    __u32 cpu;
    __u32 ifindex;
};

struct esop_network_drop_state {
    __u64 window_start_ns;
    __u32 count;
    __u32 last_reason;
};

struct esop_page_fault_key {
    __u32 cpu;
    __u32 tgid;
};

struct esop_page_fault_state {
    __u64 window_start_ns;
    __u32 count;
    __u32 reserved;
};

struct esop_cpu_frequency_state {
    __u32 max_frequency_khz;
    __u32 policy_epoch;
    __u8 below_floor;
    __u8 reserved[3];
};

struct esop_scheduler_migration_state {
    __u64 window_start_ns;
    __u32 count;
    __u32 policy_epoch;
    __u16 origin_cpu;
    __u16 destination_cpu;
    __u32 reserved;
};

struct esop_gateway_operation_key {
    __u64 request_id;
    __u32 tgid;
    __u32 reserved;
};

struct esop_gateway_operation_state {
    __u64 start_ns;
    __u32 policy_epoch;
    __u32 start_tid;
    __u32 route_kind;
    __u32 operation_class;
};

struct esop_raw_port_operation_state {
    __u64 start_ns;
    __u32 policy_epoch;
    __u32 ifindex;
    __u32 operation;
    __u32 reserved;
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
    __u8 detail;
};

_Static_assert(sizeof(struct esop_context) == 176, "context ABI changed");
_Static_assert(sizeof(struct esop_stats) == 240, "stats ABI changed");
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

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 256);
    __type(key, struct esop_page_fault_key);
    __type(value, struct esop_page_fault_state);
} ESOP_PAGE_FAULTS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1024);
    __type(key, struct esop_interrupt_key);
    __type(value, __u64);
} ESOP_IRQ_STARTS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 256);
    __type(key, struct esop_interrupt_key);
    __type(value, __u64);
} ESOP_SOFTIRQ_STARTS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 256);
    __type(key, struct esop_network_drop_key);
    __type(value, struct esop_network_drop_state);
} ESOP_NETWORK_DROPS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 256);
    __type(key, __u32);
    __type(value, struct esop_cpu_frequency_state);
} ESOP_CPU_FREQUENCY_LIMITS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 1024);
    __type(key, __u32);
    __type(value, struct esop_scheduler_migration_state);
} ESOP_SCHEDULER_MIGRATIONS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 1024);
    __type(key, struct esop_gateway_operation_key);
    __type(value, struct esop_gateway_operation_state);
} ESOP_GATEWAY_OPERATIONS SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 1024);
    __type(key, __u64);
    __type(value, struct esop_raw_port_operation_state);
} ESOP_RAW_PORT_OPERATIONS SEC(".maps");

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

static __always_inline int esop_tracks_scheduler(
    __u32 tid, const struct esop_context *context)
{
    if (!context) {
        return 0;
    }
    __u32 target = context->scheduler_tid;
    if (target == 0) {
        target = context->tracked_pid;
    }
    return target == 0 || target == tid;
}

static __always_inline int esop_emit_resource_cpu_task_id(
    __u64 evidence_id, __u8 domain, __u8 kind, __u8 severity,
    __u64 observed_value, __u64 threshold, __u64 duration_ns, __u32 count,
    __u16 irq, __u32 netdev_ifindex, __u8 detail, __u32 pid, __u32 tid,
    __u16 cpu)
{
    struct esop_context *context = esop_context();
    struct esop_stats *stats = esop_stats();
    if (!context) {
        return -1;
    }

    __u64 now = bpf_ktime_get_ns();
    __u64 timestamp = now;
    struct esop_runtime_evidence event = {};

    /* A zero producer ID falls back to the monotonic timestamp. */
    event.evidence_id = evidence_id == 0 ? timestamp : evidence_id;
    event.boot_id = context->boot_id;
    event.agent_epoch = context->agent_epoch;
    event.timestamp_ns = timestamp;
    event.cycle_seq = context->cycle_seq;
    event.transition_seq = context->transition_seq;
    event.pid = pid;
    event.tid = tid;
    event.cpu = cpu;
    event.irq = irq;
    event.netdev_ifindex = netdev_ifindex;
    event.observed_value = observed_value;
    event.threshold = threshold;
    event.duration_ns = duration_ns;
    event.count = count;
    event.domain = domain;
    event.kind = kind;
    event.severity = severity;
    event.detail = detail;
    if (bpf_ringbuf_output(&ESOP_EVENTS, &event, sizeof(event), 0) < 0) {
        if (stats) {
            stats->lost_events++;
        }
        return -1;
    }
    if (stats) {
        stats->emitted_events++;
    }
    return 0;
}

static __always_inline int esop_emit_resource_cpu_task(
    __u8 domain, __u8 kind, __u8 severity, __u64 observed_value,
    __u64 threshold, __u64 duration_ns, __u32 count, __u16 irq,
    __u32 netdev_ifindex, __u8 detail, __u32 pid, __u32 tid, __u16 cpu)
{
    return esop_emit_resource_cpu_task_id(
        0, domain, kind, severity, observed_value, threshold, duration_ns,
        count, irq, netdev_ifindex, detail, pid, tid, cpu);
}

static __always_inline int esop_emit_resource_task(
    __u8 domain, __u8 kind, __u8 severity, __u64 observed_value,
    __u64 threshold, __u64 duration_ns, __u32 count, __u16 irq,
    __u32 netdev_ifindex, __u8 detail, __u32 pid, __u32 tid)
{
    __u32 cpu = bpf_get_smp_processor_id();
    __u16 event_cpu = cpu > 0xffff ? 0xffff : (__u16)cpu;
    return esop_emit_resource_cpu_task(
        domain, kind, severity, observed_value, threshold, duration_ns, count,
        irq, netdev_ifindex, detail, pid, tid, event_cpu);
}

static __always_inline int esop_emit_resource(
    __u8 domain, __u8 kind, __u8 severity, __u64 observed_value,
    __u64 threshold, __u64 duration_ns, __u32 count, __u16 irq,
    __u32 netdev_ifindex, __u8 detail, __u8 include_task)
{
    __u32 pid = include_task ? esop_tgid() : 0;
    __u32 tid = include_task ? esop_tid() : 0;
    return esop_emit_resource_task(domain, kind, severity, observed_value,
                                   threshold, duration_ns, count, irq,
                                   netdev_ifindex, detail, pid, tid);
}

static __always_inline void esop_emit(__u8 domain, __u8 kind, __u8 severity,
                                      __u64 observed_value, __u64 threshold,
                                      __u64 duration_ns, __u32 count, __u16 irq)
{
    esop_emit_resource(domain, kind, severity, observed_value, threshold,
                       duration_ns, count, irq, 0, 0, 1);
}

static __always_inline __u16 esop_vector_u16(__u32 vector)
{
    return vector > 0xffff ? 0xffff : (__u16)vector;
}

static __always_inline __u8 esop_detail_u8(__u64 value)
{
    return value > 0xff ? 0xff : (__u8)value;
}

#define ESOP_GATEWAY_OPERATION_PUBLISH 1
#define ESOP_GATEWAY_OPERATION_CALLBACK 2
#define ESOP_GATEWAY_PUBLISH_ROUTE_MIN 0
#define ESOP_GATEWAY_PUBLISH_ROUTE_MAX 2
#define ESOP_GATEWAY_CALLBACK_ROUTE_MIN 3
#define ESOP_GATEWAY_CALLBACK_ROUTE_MAX 4
#define ESOP_GATEWAY_OUTCOME_MAX 2
#define ESOP_GATEWAY_PUBLISH_OUTCOMES 0x7
#define ESOP_GATEWAY_CALLBACK_OUTCOMES 0x5

#define ESOP_RAW_PORT_OPERATION_TX 0
#define ESOP_RAW_PORT_OPERATION_RX 1
#define ESOP_RAW_PORT_OPERATION_MAX ESOP_RAW_PORT_OPERATION_RX
#define ESOP_RAW_PORT_TX_OUTCOMES 0x7
#define ESOP_RAW_PORT_RX_OUTCOMES 0xf

static __always_inline __u8 esop_gateway_detail(__u32 route_kind,
                                                 __u32 outcome)
{
    return (__u8)((outcome << 4) | route_kind);
}

static __always_inline __u8 esop_raw_port_detail(__u32 operation,
                                                  __u32 outcome)
{
    return (__u8)((outcome << 4) | operation);
}

static __always_inline __u32 esop_skb_ifindex(struct sk_buff *skb)
{
    if (!skb) {
        return 0;
    }

    struct net_device *device = 0;
    int ifindex = 0;
    if (bpf_core_read(&device, sizeof(device), &skb->dev) == 0 && device &&
        bpf_core_read(&ifindex, sizeof(ifindex), &device->ifindex) == 0 &&
        ifindex > 0) {
        return (__u32)ifindex;
    }

    int skb_iif = 0;
    if (bpf_core_read(&skb_iif, sizeof(skb_iif), &skb->skb_iif) == 0 &&
        skb_iif > 0) {
        return (__u32)skb_iif;
    }
    return 0;
}

static __always_inline int esop_gateway_operation_begin(
    struct pt_regs *registers, __u32 operation_class, __u32 route_min,
    __u32 route_max)
{
    struct esop_context *context = esop_context();
    struct esop_stats *stats = esop_stats();
    __u64 request_id = (__u64)BPF_CORE_READ(registers, di);
    __u32 route_kind = (__u32)BPF_CORE_READ(registers, si);
    __u32 tgid = esop_tgid();
    if (!context || context->gateway_stall_threshold_ns == 0 ||
        request_id == 0 || route_kind < route_min || route_kind > route_max ||
        !esop_tracks(tgid, context)) {
        if (stats && context && esop_tracks(tgid, context)) {
            stats->gateway_probe_mismatches++;
        }
        return 0;
    }

    struct esop_gateway_operation_key key = {
        .request_id = request_id,
        .tgid = tgid,
    };
    struct esop_gateway_operation_state state = {
        .start_ns = bpf_ktime_get_ns(),
        .policy_epoch = context->gateway_probe_epoch,
        .start_tid = esop_tid(),
        .route_kind = route_kind,
        .operation_class = operation_class,
    };
    if (bpf_map_update_elem(&ESOP_GATEWAY_OPERATIONS, &key, &state,
                            BPF_NOEXIST) < 0) {
        if (stats) {
            stats->gateway_probe_mismatches++;
            stats->lost_events++;
        }
        return 0;
    }
    if (stats) {
        stats->gateway_probe_begins++;
    }
    return 0;
}

static __always_inline int esop_gateway_operation_end(
    struct pt_regs *registers, __u32 operation_class, __u32 route_min,
    __u32 route_max, __u32 allowed_outcomes)
{
    struct esop_stats *stats = esop_stats();
    __u64 request_id = (__u64)BPF_CORE_READ(registers, di);
    __u32 route_kind = (__u32)BPF_CORE_READ(registers, si);
    __u32 outcome = (__u32)BPF_CORE_READ(registers, dx);
    __u32 tgid = esop_tgid();
    if (request_id == 0) {
        if (stats) {
            stats->gateway_probe_mismatches++;
        }
        return 0;
    }

    struct esop_gateway_operation_key key = {
        .request_id = request_id,
        .tgid = tgid,
    };
    struct esop_gateway_operation_state *state =
        bpf_map_lookup_elem(&ESOP_GATEWAY_OPERATIONS, &key);
    if (!state) {
        if (stats) {
            stats->gateway_probe_mismatches++;
        }
        return 0;
    }

    __u64 start_ns = state->start_ns;
    __u32 policy_epoch = state->policy_epoch;
    __u32 start_tid = state->start_tid;
    __u32 start_route_kind = state->route_kind;
    __u32 start_operation_class = state->operation_class;
    if (bpf_map_delete_elem(&ESOP_GATEWAY_OPERATIONS, &key) < 0) {
        if (stats) {
            stats->gateway_probe_mismatches++;
        }
        return 0;
    }
    if (stats) {
        stats->gateway_probe_completions++;
    }

    struct esop_context *context = esop_context();
    if (!context || context->gateway_stall_threshold_ns == 0 ||
        route_kind < route_min || route_kind > route_max ||
        outcome > ESOP_GATEWAY_OUTCOME_MAX ||
        (allowed_outcomes & (1U << outcome)) == 0 ||
        route_kind != start_route_kind ||
        operation_class != start_operation_class ||
        policy_epoch != context->gateway_probe_epoch) {
        if (stats) {
            stats->gateway_probe_mismatches++;
        }
        return 0;
    }

    __u64 now = bpf_ktime_get_ns();
    if (now < start_ns) {
        if (stats) {
            stats->gateway_probe_mismatches++;
        }
        return 0;
    }
    __u64 duration_ns = now - start_ns;
    if (duration_ns <= context->gateway_stall_threshold_ns) {
        return 0;
    }

    __u32 end_tid = esop_tid();
    __u32 evidence_tid = start_tid == end_tid ? end_tid : 0;
    __u32 cpu = bpf_get_smp_processor_id();
    __u16 event_cpu = cpu > 0xffff ? 0xffff : (__u16)cpu;
    if (esop_emit_resource_cpu_task_id(
            request_id, 7, 7, 2, duration_ns,
            context->gateway_stall_threshold_ns, duration_ns, 1, 0, 0,
            esop_gateway_detail(route_kind, outcome), tgid, evidence_tid,
            event_cpu) == 0) {
        stats = esop_stats();
        if (stats) {
            stats->gateway_stalls++;
        }
    }
    return 0;
}

SEC("uprobe")
int esop_gateway_publish_begin(struct pt_regs *registers)
{
    return esop_gateway_operation_begin(
        registers, ESOP_GATEWAY_OPERATION_PUBLISH,
        ESOP_GATEWAY_PUBLISH_ROUTE_MIN, ESOP_GATEWAY_PUBLISH_ROUTE_MAX);
}

SEC("uprobe")
int esop_gateway_publish_end(struct pt_regs *registers)
{
    return esop_gateway_operation_end(
        registers, ESOP_GATEWAY_OPERATION_PUBLISH,
        ESOP_GATEWAY_PUBLISH_ROUTE_MIN, ESOP_GATEWAY_PUBLISH_ROUTE_MAX,
        ESOP_GATEWAY_PUBLISH_OUTCOMES);
}

SEC("uprobe")
int esop_gateway_callback_begin(struct pt_regs *registers)
{
    return esop_gateway_operation_begin(
        registers, ESOP_GATEWAY_OPERATION_CALLBACK,
        ESOP_GATEWAY_CALLBACK_ROUTE_MIN, ESOP_GATEWAY_CALLBACK_ROUTE_MAX);
}

SEC("uprobe")
int esop_gateway_callback_end(struct pt_regs *registers)
{
    return esop_gateway_operation_end(
        registers, ESOP_GATEWAY_OPERATION_CALLBACK,
        ESOP_GATEWAY_CALLBACK_ROUTE_MIN, ESOP_GATEWAY_CALLBACK_ROUTE_MAX,
        ESOP_GATEWAY_CALLBACK_OUTCOMES);
}

static __always_inline int esop_raw_port_operation_begin(
    struct pt_regs *registers)
{
    struct esop_context *context = esop_context();
    struct esop_stats *stats = esop_stats();
    __u32 ifindex = (__u32)BPF_CORE_READ(registers, di);
    __u32 operation = (__u32)BPF_CORE_READ(registers, si);
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 tgid = (__u32)(pid_tgid >> 32);
    if (!context || context->raw_port_stall_threshold_ns == 0 ||
        ifindex == 0 || operation > ESOP_RAW_PORT_OPERATION_MAX ||
        !esop_tracks(tgid, context)) {
        if (stats && context && esop_tracks(tgid, context)) {
            stats->raw_port_probe_mismatches++;
        }
        return 0;
    }

    if (bpf_map_lookup_elem(&ESOP_RAW_PORT_OPERATIONS, &pid_tgid) && stats) {
        stats->raw_port_probe_mismatches++;
    }
    struct esop_raw_port_operation_state state = {
        .start_ns = bpf_ktime_get_ns(),
        .policy_epoch = context->raw_port_probe_epoch,
        .ifindex = ifindex,
        .operation = operation,
    };
    if (bpf_map_update_elem(&ESOP_RAW_PORT_OPERATIONS, &pid_tgid, &state,
                            BPF_ANY) < 0) {
        if (stats) {
            stats->raw_port_probe_mismatches++;
            stats->lost_events++;
        }
        return 0;
    }
    if (stats) {
        stats->raw_port_probe_begins++;
    }
    return 0;
}

static __always_inline int esop_raw_port_operation_end(
    struct pt_regs *registers)
{
    struct esop_stats *stats = esop_stats();
    __u32 ifindex = (__u32)BPF_CORE_READ(registers, di);
    __u32 operation = (__u32)BPF_CORE_READ(registers, si);
    __u32 outcome = (__u32)BPF_CORE_READ(registers, dx);
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 tgid = (__u32)(pid_tgid >> 32);
    __u32 tid = (__u32)pid_tgid;
    struct esop_raw_port_operation_state *state =
        bpf_map_lookup_elem(&ESOP_RAW_PORT_OPERATIONS, &pid_tgid);
    if (!state) {
        if (stats) {
            stats->raw_port_probe_mismatches++;
        }
        return 0;
    }

    __u64 start_ns = state->start_ns;
    __u32 policy_epoch = state->policy_epoch;
    __u32 start_ifindex = state->ifindex;
    __u32 start_operation = state->operation;
    if (bpf_map_delete_elem(&ESOP_RAW_PORT_OPERATIONS, &pid_tgid) < 0) {
        if (stats) {
            stats->raw_port_probe_mismatches++;
        }
        return 0;
    }
    if (stats) {
        stats->raw_port_probe_completions++;
    }

    struct esop_context *context = esop_context();
    __u32 allowed_outcomes = operation == ESOP_RAW_PORT_OPERATION_TX
                                 ? ESOP_RAW_PORT_TX_OUTCOMES
                                 : ESOP_RAW_PORT_RX_OUTCOMES;
    if (!context || context->raw_port_stall_threshold_ns == 0 ||
        !esop_tracks(tgid, context) || ifindex == 0 ||
        operation > ESOP_RAW_PORT_OPERATION_MAX || outcome > 3 ||
        (allowed_outcomes & (1U << outcome)) == 0 ||
        ifindex != start_ifindex || operation != start_operation ||
        policy_epoch != context->raw_port_probe_epoch) {
        if (stats) {
            stats->raw_port_probe_mismatches++;
        }
        return 0;
    }

    __u64 now = bpf_ktime_get_ns();
    if (now < start_ns) {
        if (stats) {
            stats->raw_port_probe_mismatches++;
        }
        return 0;
    }
    __u64 duration_ns = now - start_ns;
    if (duration_ns <= context->raw_port_stall_threshold_ns) {
        return 0;
    }

    __u32 cpu = bpf_get_smp_processor_id();
    __u16 event_cpu = cpu > 0xffff ? 0xffff : (__u16)cpu;
    if (esop_emit_resource_cpu_task_id(
            start_ns, 5, 11, 2, duration_ns,
            context->raw_port_stall_threshold_ns, duration_ns, 1, 0,
            start_ifindex, esop_raw_port_detail(operation, outcome), tgid,
            tid, event_cpu) == 0) {
        stats = esop_stats();
        if (stats) {
            stats->raw_port_stalls++;
        }
    }
    return 0;
}

SEC("uprobe")
int esop_raw_port_begin(struct pt_regs *registers)
{
    return esop_raw_port_operation_begin(registers);
}

SEC("uprobe")
int esop_raw_port_end(struct pt_regs *registers)
{
    return esop_raw_port_operation_end(registers);
}

SEC("tracepoint/sched/sched_wakeup")
int esop_sched_wakeup(struct trace_event_raw_sched_wakeup_template *event)
{
    struct esop_context *context = esop_context();
    __u32 pid = (__u32)BPF_CORE_READ(event, pid);
    if (!esop_tracks_scheduler(pid, context)) {
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
    if (!esop_tracks_scheduler(pid, context)) {
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
        esop_emit_resource_task(0, 0, 2, latency,
                                context->scheduler_latency_threshold_ns,
                                latency, 1, 0, 0, 0, 0, pid);
        struct esop_stats *stats = esop_stats();
        if (stats) {
            stats->scheduler_stalls++;
        }
    }
    return 0;
}

SEC("tracepoint/sched/sched_migrate_task")
int esop_sched_migrate_task(struct trace_event_raw_sched_migrate_task *event)
{
    struct esop_context *context = esop_context();
    if (!context || context->scheduler_migration_threshold == 0 ||
        context->scheduler_migration_window_ns == 0) {
        return 0;
    }

    int raw_pid = BPF_CORE_READ(event, pid);
    int raw_origin = BPF_CORE_READ(event, orig_cpu);
    int raw_destination = BPF_CORE_READ(event, dest_cpu);
    if (raw_pid <= 0 || raw_origin < 0 || raw_destination < 0 ||
        raw_origin > 0xffff || raw_destination > 0xffff ||
        raw_origin == raw_destination) {
        return 0;
    }

    __u32 tid = (__u32)raw_pid;
    if (!esop_tracks_scheduler(tid, context)) {
        return 0;
    }

    struct esop_stats *stats = esop_stats();
    if (stats) {
        stats->scheduler_migrations++;
    }

    __u64 now = bpf_ktime_get_ns();
    struct esop_scheduler_migration_state *state =
        bpf_map_lookup_elem(&ESOP_SCHEDULER_MIGRATIONS, &tid);
    __u32 count = 1;
    __u64 elapsed = 0;
    int threshold_crossed = context->scheduler_migration_threshold == 1;
    if (!state) {
        struct esop_scheduler_migration_state initial = {
            .window_start_ns = now,
            .count = 1,
            .policy_epoch = context->scheduler_migration_epoch,
            .origin_cpu = (__u16)raw_origin,
            .destination_cpu = (__u16)raw_destination,
        };
        if (bpf_map_update_elem(&ESOP_SCHEDULER_MIGRATIONS, &tid, &initial,
                                BPF_ANY) < 0) {
            if (stats) {
                stats->lost_events++;
            }
            return 0;
        }
    } else if (state->policy_epoch != context->scheduler_migration_epoch ||
               now < state->window_start_ns ||
               now - state->window_start_ns >=
                   context->scheduler_migration_window_ns) {
        state->window_start_ns = now;
        state->count = 1;
        state->policy_epoch = context->scheduler_migration_epoch;
        state->origin_cpu = (__u16)raw_origin;
        state->destination_cpu = (__u16)raw_destination;
    } else {
        elapsed = now - state->window_start_ns;
        __u32 previous = state->count;
        if (state->count != 0xffffffff) {
            state->count++;
        }
        state->origin_cpu = (__u16)raw_origin;
        state->destination_cpu = (__u16)raw_destination;
        count = state->count;
        threshold_crossed =
            (__u64)previous < context->scheduler_migration_threshold &&
            (__u64)count >= context->scheduler_migration_threshold;
    }

    if (!threshold_crossed) {
        return 0;
    }

    int raw_priority = BPF_CORE_READ(event, prio);
    __u8 priority = raw_priority < 0 ? 0 : esop_detail_u8((__u64)raw_priority);
    if (esop_emit_resource_cpu_task(
            0, 10, 1, count, context->scheduler_migration_threshold,
            elapsed, count, (__u16)raw_origin, 0, priority, 0, tid,
            (__u16)raw_destination) == 0 &&
        stats) {
        stats->scheduler_migration_threshold_events++;
    }
    return 0;
}

SEC("tracepoint/sched/sched_process_exit")
int esop_process_exit(void *ctx)
{
    (void)ctx;
    struct esop_context *context = esop_context();
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 tgid = (__u32)(pid_tgid >> 32);
    __u32 tid = (__u32)pid_tgid;
    if (!esop_tracks(tgid, context)) {
        return 0;
    }

    struct esop_stats *stats = esop_stats();
    if (tid != tgid) {
        if (stats) {
            stats->thread_exits_ignored++;
        }
        return 0;
    }

    esop_emit(4, 5, 3, 1, 1, 0, 1, 0);
    if (stats) {
        stats->process_exits++;
    }
    return 0;
}

SEC("tracepoint/exceptions/page_fault_user")
int esop_page_fault_user(struct trace_event_raw_exceptions *event)
{
    struct esop_context *context = esop_context();
    __u32 tgid = esop_tgid();
    if (!esop_tracks(tgid, context)) {
        return 0;
    }

    struct esop_stats *stats = esop_stats();
    if (stats) {
        stats->page_faults++;
    }
    if (!context || context->page_fault_threshold == 0 ||
        context->page_fault_window_ns == 0) {
        return 0;
    }

    __u64 now = bpf_ktime_get_ns();
    struct esop_page_fault_key key = {
        .cpu = bpf_get_smp_processor_id(),
        .tgid = tgid,
    };
    struct esop_page_fault_state *state =
        bpf_map_lookup_elem(&ESOP_PAGE_FAULTS, &key);
    __u32 count = 1;
    __u64 elapsed = 0;
    int threshold_crossed = context->page_fault_threshold == 1;
    if (!state) {
        struct esop_page_fault_state initial = {
            .window_start_ns = now,
            .count = 1,
        };
        if (bpf_map_update_elem(&ESOP_PAGE_FAULTS, &key, &initial,
                                BPF_ANY) < 0) {
            if (stats) {
                stats->lost_events++;
            }
            return 0;
        }
    } else if (now < state->window_start_ns ||
               now - state->window_start_ns >=
                   context->page_fault_window_ns) {
        state->window_start_ns = now;
        state->count = 1;
    } else {
        elapsed = now - state->window_start_ns;
        __u32 previous = state->count;
        if (state->count != 0xffffffff) {
            state->count++;
        }
        count = state->count;
        threshold_crossed = (__u64)previous < context->page_fault_threshold &&
                            (__u64)count >= context->page_fault_threshold;
    }

    if (threshold_crossed) {
        __u64 error_code = BPF_CORE_READ(event, error_code);
        if (esop_emit_resource(3, 3, 1, count,
                               context->page_fault_threshold, elapsed,
                               count, 0, 0, esop_detail_u8(error_code), 1) ==
                0 &&
            stats) {
            stats->page_fault_threshold_events++;
        }
    }
    return 0;
}

SEC("tracepoint/oom/mark_victim")
int esop_oom_kill(struct trace_event_raw_mark_victim *event)
{
    struct esop_context *context = esop_context();
    int raw_pid = BPF_CORE_READ(event, pid);
    if (raw_pid <= 0) {
        return 0;
    }

    __u32 victim_pid = (__u32)raw_pid;
    if (!esop_tracks(victim_pid, context)) {
        return 0;
    }

    esop_emit_resource_task(3, 4, 3, 1, 1, 0, 1, 0, 0, 0,
                            victim_pid, victim_pid);
    struct esop_stats *stats = esop_stats();
    if (stats) {
        stats->oom_events++;
    }
    return 0;
}

SEC("tracepoint/skb/kfree_skb")
int esop_network_drop(struct trace_event_raw_kfree_skb *event)
{
    struct esop_context *context = esop_context();
    if (!context) {
        return 0;
    }

    __u16 protocol = (__u16)BPF_CORE_READ(event, protocol);
    if (protocol != context->network_protocol) {
        return 0;
    }

    struct sk_buff *skb = (struct sk_buff *)BPF_CORE_READ(event, skbaddr);
    __u32 ifindex = esop_skb_ifindex(skb);
    struct esop_stats *stats = esop_stats();
    if (ifindex == 0) {
        if (stats) {
            stats->network_unattributed++;
        }
        return 0;
    }
    if (context->network_ifindex != 0 &&
        context->network_ifindex != ifindex) {
        return 0;
    }
    if (stats) {
        stats->network_drops++;
    }

    int raw_reason = BPF_CORE_READ(event, reason);
    __u32 reason = raw_reason < 0 ? 0 : (__u32)raw_reason;
    __u64 now = bpf_ktime_get_ns();
    struct esop_network_drop_key key = {
        .cpu = bpf_get_smp_processor_id(),
        .ifindex = ifindex,
    };
    struct esop_network_drop_state *state =
        bpf_map_lookup_elem(&ESOP_NETWORK_DROPS, &key);
    __u32 count = 1;
    __u64 elapsed = 0;
    if (!state) {
        struct esop_network_drop_state initial = {
            .window_start_ns = now,
            .count = 1,
            .last_reason = reason,
        };
        if (bpf_map_update_elem(&ESOP_NETWORK_DROPS, &key, &initial,
                                BPF_ANY) < 0) {
            stats = esop_stats();
            if (stats) {
                stats->lost_events++;
            }
            return 0;
        }
    } else if (now < state->window_start_ns ||
               now - state->window_start_ns >=
                   context->network_drop_window_ns) {
        state->window_start_ns = now;
        state->count = 1;
        state->last_reason = reason;
    } else {
        elapsed = now - state->window_start_ns;
        if (state->count != 0xffffffff) {
            state->count++;
        }
        state->last_reason = reason;
        count = state->count;
    }

    if ((__u64)count == context->network_drop_threshold) {
        if (esop_emit_resource(2, 2, 2, count,
                               context->network_drop_threshold, elapsed,
                               count, 0, ifindex, esop_detail_u8(reason), 0) ==
                0 &&
            stats) {
            stats->network_threshold_events++;
        }
    }
    return 0;
}

SEC("tracepoint/power/cpu_frequency_limits")
int esop_cpu_frequency_limit(
    struct trace_event_raw_cpu_frequency_limits *event)
{
    struct esop_context *context = esop_context();
    if (!context || context->cpu_frequency_floor_khz == 0) {
        return 0;
    }

    __u32 policy_cpu = BPF_CORE_READ(event, cpu_id);
    if (context->cpu_frequency_policy_cpu != 0xffffffff &&
        context->cpu_frequency_policy_cpu != policy_cpu) {
        return 0;
    }

    struct esop_stats *stats = esop_stats();
    if (stats) {
        stats->cpu_frequency_updates++;
    }

    __u32 max_frequency_khz = BPF_CORE_READ(event, max_freq);
    if (max_frequency_khz == 0) {
        return 0;
    }

    __u32 floor_khz = context->cpu_frequency_floor_khz;
    __u8 below_floor = max_frequency_khz < floor_khz;
    struct esop_cpu_frequency_state *state =
        bpf_map_lookup_elem(&ESOP_CPU_FREQUENCY_LIMITS, &policy_cpu);
    if (!state) {
        struct esop_cpu_frequency_state initial = {
            .max_frequency_khz = max_frequency_khz,
            .policy_epoch = context->cpu_frequency_policy_epoch,
            .below_floor = below_floor,
        };
        if (bpf_map_update_elem(&ESOP_CPU_FREQUENCY_LIMITS, &policy_cpu,
                                &initial, BPF_ANY) < 0) {
            if (stats) {
                stats->lost_events++;
            }
            return 0;
        }
        if (!below_floor) {
            return 0;
        }
    } else {
        if (state->policy_epoch != context->cpu_frequency_policy_epoch) {
            state->policy_epoch = context->cpu_frequency_policy_epoch;
            state->below_floor = 0;
        }
        state->max_frequency_khz = max_frequency_khz;
        if (below_floor) {
            if (state->below_floor) {
                if (stats) {
                    stats->cpu_frequency_suppressed++;
                }
                return 0;
            }
            state->below_floor = 1;
        } else {
            if (state->below_floor && stats) {
                stats->cpu_frequency_recoveries++;
            }
            state->below_floor = 0;
            return 0;
        }
    }

    __u16 event_cpu = policy_cpu > 0xffff ? 0xffff : (__u16)policy_cpu;
    if (esop_emit_resource_cpu_task(0, 6, 2, max_frequency_khz, floor_khz,
                                    0, 1, 0, 0, 0, 0, 0,
                                    event_cpu) == 0 &&
        stats) {
        stats->cpu_frequency_throttle_events++;
    }
    return 0;
}

SEC("tracepoint/irq/irq_handler_entry")
int esop_irq_handler_entry(struct trace_event_raw_irq_handler_entry *event)
{
    int irq = BPF_CORE_READ(event, irq);
    if (irq < 0) {
        return 0;
    }
    struct esop_interrupt_key key = {
        .cpu = bpf_get_smp_processor_id(),
        .vector = (__u32)irq,
    };
    __u64 now = bpf_ktime_get_ns();
    if (bpf_map_update_elem(&ESOP_IRQ_STARTS, &key, &now, BPF_ANY) < 0) {
        struct esop_stats *stats = esop_stats();
        if (stats) {
            stats->lost_events++;
        }
    }
    return 0;
}

SEC("tracepoint/irq/irq_handler_exit")
int esop_irq_handler_exit(struct trace_event_raw_irq_handler_exit *event)
{
    int irq = BPF_CORE_READ(event, irq);
    if (irq < 0) {
        return 0;
    }
    struct esop_interrupt_key key = {
        .cpu = bpf_get_smp_processor_id(),
        .vector = (__u32)irq,
    };
    __u64 *start = bpf_map_lookup_elem(&ESOP_IRQ_STARTS, &key);
    if (!start) {
        return 0;
    }
    __u64 start_ns = *start;
    bpf_map_delete_elem(&ESOP_IRQ_STARTS, &key);
    __u64 now = bpf_ktime_get_ns();
    if (now < start_ns) {
        return 0;
    }
    __u64 duration = now - start_ns;
    struct esop_stats *stats = esop_stats();
    if (stats) {
        stats->irq_samples++;
    }
    struct esop_context *context = esop_context();
    __u64 threshold = context ? context->irq_duration_threshold_ns : 0;
    if (context && duration > threshold) {
        esop_emit(1, 1, 2, duration, threshold,
                  duration, 1, esop_vector_u16((__u32)irq));
        stats = esop_stats();
        if (stats) {
            stats->irq_overruns++;
        }
    }
    return 0;
}

SEC("tracepoint/irq/softirq_entry")
int esop_softirq_entry(struct trace_event_raw_softirq *event)
{
    __u32 vector = (__u32)BPF_CORE_READ(event, vec);
    struct esop_interrupt_key key = {
        .cpu = bpf_get_smp_processor_id(),
        .vector = vector,
    };
    __u64 now = bpf_ktime_get_ns();
    if (bpf_map_update_elem(&ESOP_SOFTIRQ_STARTS, &key, &now, BPF_ANY) < 0) {
        struct esop_stats *stats = esop_stats();
        if (stats) {
            stats->lost_events++;
        }
    }
    return 0;
}

SEC("tracepoint/irq/softirq_exit")
int esop_softirq_exit(struct trace_event_raw_softirq *event)
{
    __u32 vector = (__u32)BPF_CORE_READ(event, vec);
    struct esop_interrupt_key key = {
        .cpu = bpf_get_smp_processor_id(),
        .vector = vector,
    };
    __u64 *start = bpf_map_lookup_elem(&ESOP_SOFTIRQ_STARTS, &key);
    if (!start) {
        return 0;
    }
    __u64 start_ns = *start;
    bpf_map_delete_elem(&ESOP_SOFTIRQ_STARTS, &key);
    __u64 now = bpf_ktime_get_ns();
    if (now < start_ns) {
        return 0;
    }
    __u64 duration = now - start_ns;
    struct esop_stats *stats = esop_stats();
    if (stats) {
        stats->softirq_samples++;
    }
    struct esop_context *context = esop_context();
    __u64 threshold = context ? context->softirq_duration_threshold_ns : 0;
    if (context && duration > threshold) {
        esop_emit(1, 9, 2, duration, threshold,
                  duration, 1, esop_vector_u16(vector));
        stats = esop_stats();
        if (stats) {
            stats->softirq_overruns++;
        }
    }
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
