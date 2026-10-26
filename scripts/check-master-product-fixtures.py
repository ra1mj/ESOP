#!/usr/bin/env python3
"""Regenerate all software master products and compare their six frozen artifacts."""
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ("device_inventory.json", "esop_product_config.h", "esop_product_config.rs",
             "procbuf_layout.json", "product_config.json", "robot_build_input.json")

# Consume the public C interface as an application does. Compiling a header as
# its own translation unit makes Clang correctly report its internal tables as
# unused; suppressing that diagnostic would miss an actual consumer contract.
C_CONSUMER = r'''#include "esop_product_config.h"
#include <stddef.h>
#include <stdio.h>
#include <string.h>

typedef struct {
    const esop_slave_config_t *slaves;
    size_t slave_count;
    const esop_domain_config_t *domains;
    size_t domain_count;
    const esop_pdo_config_t *pdos;
    size_t pdo_count;
    const esop_datagram_config_t *datagrams;
    size_t datagram_count;
    const esop_slave_copy_config_t *copies;
    size_t copy_count;
    const esop_sdo_information_expectation_t *sdo_information;
    size_t sdo_information_count;
    const esop_mailbox_status_mapping_t *mailbox_status;
    const esop_axis_config_t *axes;
    size_t axis_count;
} product_view_t;

#define REQUIRE(condition) do { \
    if (!(condition)) { \
        fprintf(stderr, "generated C configuration: %s\n", #condition); \
        return 1; \
    } \
} while (0)

static const esop_domain_config_t *domain_for(const product_view_t *p, uint8_t id) {
    for (size_t i = 0; i < p->domain_count; ++i) {
        if (p->domains[i].id == id) return &p->domains[i];
    }
    return NULL;
}

static const esop_slave_config_t *slave_for(const product_view_t *p, uint16_t position) {
    for (size_t i = 0; i < p->slave_count; ++i) {
        if (p->slaves[i].position == position) return &p->slaves[i];
    }
    return NULL;
}

static int consume_product(const product_view_t *p) {
    for (size_t i = 0; i < p->slave_count; ++i) {
        const esop_slave_config_t *slave = &p->slaves[i];
        REQUIRE(slave->name != NULL && slave->name[0] != '\0');
        REQUIRE(slave->station_address != 0);
        REQUIRE(domain_for(p, slave->domain_id) != NULL);
        REQUIRE(slave->sii_fmmu_count <= ESOP_SII_FMMU_CAPACITY);
        REQUIRE((uint64_t)slave->sdo_information_offset + slave->sdo_information_count
                <= p->sdo_information_count);
        const esop_mailbox_status_mapping_t *status = &p->mailbox_status[i];
        REQUIRE(status->slave_position == slave->position);
        if (status->present) {
            const esop_domain_config_t *domain = domain_for(p, status->domain_id);
            REQUIRE(domain != NULL && status->enable != 0);
            REQUIRE(status->domain_bit_offset < (uint64_t)domain->image_bytes * 8);
        }
    }
    for (size_t i = 0; i < p->domain_count; ++i) {
        const esop_domain_config_t *domain = &p->domains[i];
        REQUIRE(domain->name != NULL && domain->image_bytes != 0);
        REQUIRE((uint64_t)domain->output_bytes + domain->input_bytes == domain->image_bytes);
        REQUIRE(domain->period_ticks != 0 && domain->phase_ticks < domain->period_ticks);
    }
    for (size_t i = 0; i < p->pdo_count; ++i) {
        const esop_pdo_config_t *pdo = &p->pdos[i];
        const esop_domain_config_t *domain = domain_for(p, pdo->domain_id);
        const esop_slave_config_t *slave = slave_for(p, pdo->slave_position);
        REQUIRE(domain != NULL && slave != NULL);
        REQUIRE(slave->domain_id == pdo->domain_id);
        REQUIRE(pdo->bit_length != 0 && pdo->bit_length <= 64);
        REQUIRE((uint64_t)pdo->bit_offset + pdo->bit_length
                <= (uint64_t)domain->image_bytes * 8);
    }
    for (size_t i = 0; i < p->datagram_count; ++i) {
        const esop_datagram_config_t *datagram = &p->datagrams[i];
        const esop_domain_config_t *domain = domain_for(p, datagram->domain_id);
        REQUIRE(domain != NULL && datagram->payload_len != 0);
        REQUIRE(datagram->image_offset >= domain->image_offset);
        REQUIRE((uint64_t)datagram->image_offset + datagram->payload_len
                <= (uint64_t)domain->image_offset + domain->image_bytes);
    }
    for (size_t i = 0; i < p->copy_count; ++i) {
        const esop_slave_copy_config_t *copy = &p->copies[i];
        REQUIRE(copy->name != NULL);
        REQUIRE(copy->source_pdo_index < p->pdo_count);
        REQUIRE(copy->target_pdo_index < p->pdo_count);
        REQUIRE(copy->target_quality_pdo_index < p->pdo_count);
    }
    for (size_t i = 0; i < p->sdo_information_count; ++i) {
        const esop_sdo_information_expectation_t *entry = &p->sdo_information[i];
        REQUIRE(slave_for(p, entry->slave_position) != NULL);
        REQUIRE(entry->bit_length != 0 && entry->required_access != 0);
    }
    for (size_t i = 0; i < p->axis_count; ++i) {
        const esop_axis_config_t *axis = &p->axes[i];
        REQUIRE(axis->name != NULL && axis->index == i);
        REQUIRE(slave_for(p, axis->slave_position) != NULL);
        REQUIRE(axis->mode == 8 || axis->mode == 9 || axis->mode == 10);
        REQUIRE(axis->position_scale != 0 && axis->velocity_scale != 0
                && axis->torque_scale != 0);
    }
    return 0;
}

int main(void) {
    const product_view_t product = {
        esop_slaves, ESOP_SLAVE_COUNT, esop_domains, ESOP_DOMAIN_COUNT,
        esop_pdos, ESOP_PDO_COUNT, esop_datagrams, ESOP_DATAGRAM_COUNT,
        esop_slave_copies, ESOP_SLAVE_COPY_COUNT,
        esop_sdo_information_expectations, ESOP_SDO_INFORMATION_COUNT,
        esop_mailbox_status_mappings, esop_axes, ESOP_AXIS_COUNT
    };
    REQUIRE(ESOP_PRODUCT_NAME[0] != '\0' && strlen(ESOP_CONFIG_SHA256) == 64);
    REQUIRE(ESOP_ROBOT_ID != 0 && ESOP_POLICY_VERSION != 0);
    REQUIRE(ESOP_RUNTIME_SDO_REQUESTS != 0 && ESOP_MAX_SDO_OBJECT_BYTES != 0);
    REQUIRE(esop_procbuf_abi_version != 0 && esop_procbuf_region_bytes != 0);
    REQUIRE(esop_procbuf_layout_hash != 0);
    return consume_product(&product);
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"))
    args = parser.parse_args()
    products = [(f"dual-{mode}", ROOT / "config/examples/sim-r0-r2" /
                 ("product.json" if mode == "csp" else f"product-{mode}.json"),
                 ROOT / f"config/examples/sim-r0-r2/expected/{mode}")
                for mode in ("csp", "csv", "cst")]
    products += [(grade, ROOT / f"config/examples/sim-r0-r2-load/{grade}/product.json",
                  ROOT / f"config/examples/sim-r0-r2-load/{grade}/expected")
                 for grade in ("q1", "q2", "q4")]
    for name, product, expected in products:
        output = ROOT / "build/generated/master" / name
        subprocess.run([args.cargo, "run", "--quiet", "-p", "esop-cfggen", "--",
                        "--input", str(product), "--output", str(output)], cwd=ROOT, check=True)
        for artifact in ARTIFACTS:
            if (output / artifact).read_bytes() != (expected / artifact).read_bytes():
                raise SystemExit(f"stale master product: {expected / artifact}")
        consumer = output / "consume-product-config.c"
        executable = output / "consume-product-config"
        consumer.write_text(C_CONSUMER, encoding="utf-8")
        subprocess.run(["gcc", "-std=c11", "-Wall", "-Wextra", "-Werror",
                        str(consumer), "-o", str(executable)], check=True)
        subprocess.run([str(executable)], check=True)
    print("all six software master product sets match their frozen artifacts")


if __name__ == "__main__":
    main()
