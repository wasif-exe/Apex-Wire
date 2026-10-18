#include <linux/bpf.h>
#include <linux/in.h>
#include <linux/if_ether.h>
#include <linux/ip.h>
#include <linux/tcp.h>
#include <linux/udp.h>
#include <bpf/bpf_helpers.h>
struct risk_limits {
    __u32 max_order_size;
    __u32 min_price;
    __u32 max_price;
    __u32 max_notional;
    __u8 enabled;
    __u8 _pad[3];
};
struct symbol_key {
    __u64 symbol;
};
struct risk_stats {
    __u64 passed_count;
    __u64 dropped_size;
    __u64 dropped_price;
    __u64 dropped_symbol;
    __u64 dropped_notional;
};
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1024);
    __uint(key_size, sizeof(struct symbol_key));
    __uint(value_size, sizeof(struct risk_limits));
} symbol_risk_map SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(struct risk_stats));
} risk_stats_map SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_XSKMAP);
    __uint(max_entries, 64);
    __uint(key_size, sizeof(int));
    __uint(value_size, sizeof(int));
} xsks_map SEC(".maps");
SEC("xdp")
int xdp_pretrade_risk(struct xdp_md *ctx) {
    void *data_end = (void *)(long)ctx->data_end;
    void *data = (void *)(long)ctx->data;
    struct ethhdr *eth = data;
    if ((void *)(eth + 1) > data_end) {
        return XDP_PASS;
    }
    if (eth->h_proto != __constant_htons(ETH_P_IP)) {
        return XDP_PASS;
    }
    struct iphdr *iph = (void *)(eth + 1);
    if ((void *)(iph + 1) > data_end) {
        return XDP_PASS;
    }
    void *payload = NULL;
    __u32 payload_len = 0;
    if (iph->protocol == IPPROTO_TCP) {
        struct tcphdr *th = (void *)((__u32 *)iph + iph->ihl);
        if ((void *)(th + 1) > data_end) {
            return XDP_PASS;
        }
        payload = (void *)((__u32 *)th + th->doff);
    } else if (iph->protocol == IPPROTO_UDP) {
        struct udphdr *uh = (void *)((__u32 *)iph + iph->ihl);
        if ((void *)(uh + 1) > data_end) {
            return XDP_PASS;
        }
        payload = (void *)(uh + 1);
    } else {
        return XDP_PASS;
    }
    if (payload > data_end) {
        return XDP_PASS;
    }
    payload_len = (__u32)(data_end - payload);
    if (payload_len >= 36) {
        __u8 *msg = payload;
        if ((void *)(msg + 36) > data_end) {
            return XDP_PASS;
        }
        if (msg[0] == 'A') {
            __u32 shares = (__u32)msg[20] << 24 | (__u32)msg[21] << 16 | (__u32)msg[22] << 8 | (__u32)msg[23];
            __u64 sym = *(__u64 *)(msg + 24);
            __u32 price = (__u32)msg[32] << 24 | (__u32)msg[33] << 16 | (__u32)msg[34] << 8 | (__u32)msg[35];
            struct symbol_key sk;
            sk.symbol = sym;
            struct risk_limits *limits = bpf_map_lookup_elem(&symbol_risk_map, &sk);
            __u32 zero = 0;
            struct risk_stats *st = bpf_map_lookup_elem(&risk_stats_map, &zero);
            if (limits && limits->enabled) {
                if (limits->max_order_size > 0 && shares > limits->max_order_size) {
                    if (st) { st->dropped_size++; }
                    return XDP_DROP;
                }
                if (limits->min_price > 0 && price < limits->min_price) {
                    if (st) { st->dropped_price++; }
                    return XDP_DROP;
                }
                if (limits->max_price > 0 && price > limits->max_price) {
                    if (st) { st->dropped_price++; }
                    return XDP_DROP;
                }
                __u64 notional = (__u64)shares * (__u64)price;
                if (limits->max_notional > 0 && notional > (__u64)limits->max_notional) {
                    if (st) { st->dropped_notional++; }
                    return XDP_DROP;
                }
            }
            if (st) { st->passed_count++; }
        }
    }
    int index = ctx->rx_queue_index;
    if (bpf_map_lookup_elem(&xsks_map, &index)) {
        return bpf_redirect_map(&xsks_map, index, 0);
    }
    return XDP_PASS;
}
char _license[] SEC("license") = "GPL";
