#ifndef ESOP_PRODUCT_CONFIG_H
#define ESOP_PRODUCT_CONFIG_H

#include <stdint.h>

#define ESOP_SII_FMMU_CAPACITY 16u

typedef struct { const char *name; uint16_t position; uint16_t station_address; uint8_t domain_id; uint8_t kind; uint32_t vendor_id; uint32_t product_code; uint32_t revision; uint32_t serial; uint8_t has_serial; uint8_t dc_required; uint8_t dc_reference_clock; const char *dc_op_mode; uint8_t has_dc_op_mode; uint32_t dc_cycle_time0_ns; int32_t dc_shift_time0_ns; int32_t dc_shift_time1_ns; int16_t dc_sync1_cycle_factor; uint16_t dc_assign_activate; int16_t dc_sync0_cycle_factor; uint8_t has_dc_sync_timing; uint32_t dc_sync_cycle_time0_ns; uint32_t dc_sync_cycle_time1_ns; int32_t dc_sync_shift_time0_ns; uint16_t dc_sync_assign_activate; uint16_t mailbox_send_address; uint16_t mailbox_send_capacity; uint16_t mailbox_receive_address; uint16_t mailbox_receive_capacity; uint8_t mailbox_send_sync_manager; uint8_t mailbox_send_control_byte; uint8_t mailbox_receive_sync_manager; uint8_t mailbox_receive_control_byte; uint8_t has_mailbox_status_bit; uint16_t mailbox_status_bit_address; uint8_t mailbox_status_bit_mask; uint8_t mailbox_status_bit_active_high; uint8_t sii_sync_manager_count; uint16_t sii_enabled_sync_managers; uint8_t sii_fmmu_count; uint8_t sii_fmmu_usages[ESOP_SII_FMMU_CAPACITY]; uint8_t has_watchdog; uint8_t has_watchdog_divider; uint16_t watchdog_divider; uint8_t has_process_data_watchdog; uint16_t process_data_watchdog_intervals; uint8_t coe_complete_access_supported; uint8_t coe_complete_access_enabled; } esop_slave_config_t;
typedef struct { const char *name; uint8_t id; uint32_t logical_address; uint32_t image_offset; uint32_t image_bytes; uint32_t output_bytes; uint32_t input_bytes; uint32_t period_ticks; uint32_t phase_ticks; uint16_t expected_wkc; } esop_domain_config_t;
typedef struct { uint16_t slave_position; uint8_t present; uint8_t domain_id; uint32_t domain_bit_offset; uint32_t max_age_cycles; uint8_t fmmu_index; uint32_t logical_start; uint8_t logical_start_bit; uint8_t logical_end_bit; uint16_t physical_start; uint8_t physical_start_bit; uint8_t fmmu_type; uint8_t enable; } esop_mailbox_status_mapping_t;
typedef struct { uint8_t domain_id; uint16_t slave_position; uint16_t assignment_index; uint8_t sync_manager; uint16_t object_index; uint8_t subindex; uint8_t direction; uint32_t bit_offset; uint8_t bit_length; uint8_t is_signed; } esop_pdo_config_t;
typedef struct { uint8_t domain_id; uint8_t command; uint8_t index; uint32_t logical_address; uint32_t image_offset; uint16_t payload_len; uint16_t expected_wkc; uint8_t input; } esop_datagram_config_t;
typedef struct { const char *name; uint32_t source_pdo_index; uint32_t target_pdo_index; uint32_t target_quality_pdo_index; uint8_t invalid_fill; } esop_slave_copy_config_t;
typedef struct { const char *name; uint8_t index; uint16_t slave_position; int8_t mode; double position_scale; double velocity_scale; double torque_scale; int32_t position_offset; double min_position; double max_position; double max_velocity; double max_torque; double max_position_step; } esop_axis_config_t;

#define ESOP_PRODUCT_NAME "ESOP dual-axis simulator"
#define ESOP_CONFIG_SHA256 "4fb4315ab4c089fa56d9c8bf979bb37e9568b2ca3c7be1ead4ae1f45dd922f52"
#define ESOP_ROBOT_ID UINT64_C(0x000000000000e502)
#define ESOP_POLICY_VERSION UINT32_C(1)
#define ESOP_SLAVE_COUNT 3u
#define ESOP_DOMAIN_COUNT 2u
#define ESOP_PDO_COUNT 18u
#define ESOP_DATAGRAM_COUNT 4u
#define ESOP_SLAVE_COPY_COUNT 1u
#define ESOP_AXIS_COUNT 2u
#define ESOP_SLAVE_STORAGE_COUNT (ESOP_SLAVE_COUNT ? ESOP_SLAVE_COUNT : 1u)
#define ESOP_DOMAIN_STORAGE_COUNT (ESOP_DOMAIN_COUNT ? ESOP_DOMAIN_COUNT : 1u)
#define ESOP_PDO_STORAGE_COUNT (ESOP_PDO_COUNT ? ESOP_PDO_COUNT : 1u)
#define ESOP_DATAGRAM_STORAGE_COUNT (ESOP_DATAGRAM_COUNT ? ESOP_DATAGRAM_COUNT : 1u)
#define ESOP_SLAVE_COPY_STORAGE_COUNT (ESOP_SLAVE_COPY_COUNT ? ESOP_SLAVE_COPY_COUNT : 1u)
#define ESOP_AXIS_STORAGE_COUNT (ESOP_AXIS_COUNT ? ESOP_AXIS_COUNT : 1u)

static const uint16_t esop_procbuf_abi_version = 6u;
static const uint32_t esop_procbuf_region_bytes = 4144u;
static const uint64_t esop_procbuf_layout_hash = UINT64_C(0x9f41a7c67a7a6376);

static const esop_slave_config_t esop_slaves[ESOP_SLAVE_STORAGE_COUNT] = {
  {"drive_left", 0u, UINT16_C(0x1001), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004020), UINT32_C(0x00000001), UINT32_C(0x00000001), 1u, 1u, 1u, "DcSync", 1u, UINT32_C(1000000), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u},
  {"drive_right", 1u, UINT16_C(0x1002), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004020), UINT32_C(0x00000001), UINT32_C(0x00000002), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(1000000), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 0u},
  {"io_block", 2u, UINT16_C(0x1003), 1u, 2u, UINT32_C(0x0000e500), UINT32_C(0x00007010), UINT32_C(0x00000001), UINT32_C(0x00000000), 0u, 0u, 0u, 0, 0u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0000), INT16_C(0), 0u, UINT32_C(0), UINT32_C(0), INT32_C(0), UINT16_C(0x0000), UINT16_C(0x1200), UINT16_C(32), UINT16_C(0x1300), UINT16_C(32), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 0u, UINT16_C(0x0000), UINT8_C(0x00), 0u, 4u, UINT16_C(0x000f), 2u, {1u, 2u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 0u, 0u, UINT16_C(0), 0u, UINT16_C(0), 0u, 0u},
};

static const esop_mailbox_status_mapping_t esop_mailbox_status_mappings[ESOP_SLAVE_STORAGE_COUNT] = {
  {0u, 1u, 0u, 256u, 1u, 2u, UINT32_C(0x00001020), 0u, 0u, UINT16_C(0x080d), 3u, 1u, 1u},
  {1u, 1u, 0u, 257u, 1u, 2u, UINT32_C(0x00001020), 1u, 1u, UINT16_C(0x080d), 3u, 1u, 1u},
  {2u, 0u, 0u, 0u, 0u, 0u, UINT32_C(0x00000000), 0u, 0u, UINT16_C(0x0000), 0u, 0u, 0u},
};

static const esop_domain_config_t esop_domains[ESOP_DOMAIN_STORAGE_COUNT] = {
  {"motion", 0u, UINT32_C(0x00001000), 0u, 33u, 14u, 19u, 1u, 0u, 6u},
  {"io", 1u, UINT32_C(0x00001100), 64u, 9u, 7u, 2u, 4u, 0u, 2u},
};

static const esop_pdo_config_t esop_pdos[ESOP_PDO_STORAGE_COUNT] = {
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 0u, 16u, 0u},
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 16u, 8u, 1u},
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 24u, 32u, 1u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 56u, 16u, 0u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 72u, 8u, 1u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 80u, 32u, 1u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 112u, 16u, 0u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 128u, 8u, 1u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 136u, 16u, 0u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 152u, 32u, 1u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 184u, 16u, 0u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 200u, 8u, 1u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 208u, 16u, 0u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 224u, 32u, 1u},
  {1u, 2u, UINT16_C(0x1601), 2u, UINT16_C(0x7000), 1u, 0u, 0u, 16u, 0u},
  {1u, 2u, UINT16_C(0x1601), 2u, UINT16_C(0x7010), 1u, 0u, 16u, 32u, 1u},
  {1u, 2u, UINT16_C(0x1601), 2u, UINT16_C(0x7011), 1u, 0u, 48u, 8u, 0u},
  {1u, 2u, UINT16_C(0x1a01), 3u, UINT16_C(0x6000), 1u, 1u, 56u, 16u, 0u},
};

static const esop_datagram_config_t esop_datagrams[ESOP_DATAGRAM_STORAGE_COUNT] = {
  {0u, UINT8_C(0x0b), 0u, UINT32_C(0x00001000), 0u, 14u, 2u, 0u},
  {0u, UINT8_C(0x0a), 1u, UINT32_C(0x0000100e), 14u, 19u, 4u, 1u},
  {1u, UINT8_C(0x0b), 2u, UINT32_C(0x00001100), 64u, 7u, 1u, 0u},
  {1u, UINT8_C(0x0a), 3u, UINT32_C(0x00001107), 71u, 2u, 1u, 1u},
};

static const esop_slave_copy_config_t esop_slave_copies[ESOP_SLAVE_COPY_STORAGE_COUNT] = {
  {"left_position_to_io", 9u, 15u, 16u, UINT8_C(0x00)},
};

static const esop_axis_config_t esop_axes[ESOP_AXIS_STORAGE_COUNT] = {
  {"left_joint", 0u, 0u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"right_joint", 1u, 1u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
};

#endif
