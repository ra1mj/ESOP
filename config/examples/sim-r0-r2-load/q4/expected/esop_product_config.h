#ifndef ESOP_PRODUCT_CONFIG_H
#define ESOP_PRODUCT_CONFIG_H

#include <stdint.h>

#define ESOP_SII_FMMU_CAPACITY 16u

#define ESOP_RUNTIME_SDO_REQUESTS 8u
#define ESOP_MAX_SDO_OBJECT_BYTES 1024u

typedef struct { const char *name; uint16_t position; uint16_t station_address; uint8_t domain_id; uint8_t kind; uint32_t vendor_id; uint32_t product_code; uint32_t revision; uint32_t serial; uint8_t has_serial; uint8_t dc_required; uint8_t dc_reference_clock; const char *dc_op_mode; uint8_t has_dc_op_mode; uint32_t dc_cycle_time0_ns; int32_t dc_shift_time0_ns; int32_t dc_shift_time1_ns; int16_t dc_sync1_cycle_factor; uint16_t dc_assign_activate; int16_t dc_sync0_cycle_factor; uint8_t has_dc_sync_timing; uint32_t dc_sync_cycle_time0_ns; uint32_t dc_sync_cycle_time1_ns; int32_t dc_sync_shift_time0_ns; uint16_t dc_sync_assign_activate; uint16_t mailbox_send_address; uint16_t mailbox_send_capacity; uint16_t mailbox_receive_address; uint16_t mailbox_receive_capacity; uint8_t mailbox_send_sync_manager; uint8_t mailbox_send_control_byte; uint8_t mailbox_receive_sync_manager; uint8_t mailbox_receive_control_byte; uint8_t has_mailbox_status_bit; uint16_t mailbox_status_bit_address; uint8_t mailbox_status_bit_mask; uint8_t mailbox_status_bit_active_high; uint8_t sii_sync_manager_count; uint16_t sii_enabled_sync_managers; uint8_t sii_fmmu_count; uint8_t sii_fmmu_usages[ESOP_SII_FMMU_CAPACITY]; uint8_t has_watchdog; uint8_t has_watchdog_divider; uint16_t watchdog_divider; uint8_t has_process_data_watchdog; uint16_t process_data_watchdog_intervals; uint8_t requesting_id_supported; uint8_t has_requesting_id; uint16_t requesting_id; uint8_t coe_complete_access_supported; uint8_t coe_complete_access_enabled; uint8_t coe_sdo_information_supported; uint8_t coe_sdo_information_enabled; uint32_t sdo_information_offset; uint32_t sdo_information_count; } esop_slave_config_t;
typedef struct { uint16_t slave_position; uint16_t index; uint8_t subindex; uint16_t data_type; uint16_t bit_length; uint8_t required_access; } esop_sdo_information_expectation_t;
typedef struct { const char *name; uint8_t id; uint32_t logical_address; uint32_t image_offset; uint32_t image_bytes; uint32_t output_bytes; uint32_t input_bytes; uint32_t period_ticks; uint32_t phase_ticks; uint16_t expected_wkc; } esop_domain_config_t;
typedef struct { uint16_t slave_position; uint8_t present; uint8_t domain_id; uint32_t domain_bit_offset; uint32_t max_age_cycles; uint8_t fmmu_index; uint32_t logical_start; uint8_t logical_start_bit; uint8_t logical_end_bit; uint16_t physical_start; uint8_t physical_start_bit; uint8_t fmmu_type; uint8_t enable; } esop_mailbox_status_mapping_t;
typedef struct { uint8_t domain_id; uint16_t slave_position; uint16_t assignment_index; uint8_t sync_manager; uint16_t object_index; uint8_t subindex; uint8_t direction; uint32_t bit_offset; uint8_t bit_length; uint8_t is_signed; } esop_pdo_config_t;
typedef struct { uint8_t domain_id; uint8_t command; uint8_t index; uint32_t logical_address; uint32_t image_offset; uint16_t payload_len; uint16_t expected_wkc; uint8_t input; } esop_datagram_config_t;
typedef struct { const char *name; uint32_t source_pdo_index; uint32_t target_pdo_index; uint32_t target_quality_pdo_index; uint8_t invalid_fill; } esop_slave_copy_config_t;
typedef struct { const char *name; uint8_t index; uint16_t slave_position; int8_t mode; double position_scale; double velocity_scale; double torque_scale; int32_t position_offset; double min_position; double max_position; double max_velocity; double max_torque; double max_position_step; } esop_axis_config_t;

#define ESOP_PRODUCT_NAME "ESOP Q4 software load fixture"
#define ESOP_CONFIG_SHA256 "d7d90eb51c4d8014d79284ef6e053df3f22a9223eae21f827cedd27940e83320"
#define ESOP_ROBOT_ID UINT64_C(0x000000000000e524)
#define ESOP_POLICY_VERSION UINT32_C(1)
#define ESOP_SLAVE_COUNT 17u
#define ESOP_DOMAIN_COUNT 2u
#define ESOP_PDO_COUNT 180u
#define ESOP_DATAGRAM_COUNT 4u
#define ESOP_SLAVE_COPY_COUNT 0u
#define ESOP_SDO_INFORMATION_COUNT 0u
#define ESOP_AXIS_COUNT 16u
#define ESOP_SLAVE_STORAGE_COUNT (ESOP_SLAVE_COUNT ? ESOP_SLAVE_COUNT : 1u)
#define ESOP_DOMAIN_STORAGE_COUNT (ESOP_DOMAIN_COUNT ? ESOP_DOMAIN_COUNT : 1u)
#define ESOP_PDO_STORAGE_COUNT (ESOP_PDO_COUNT ? ESOP_PDO_COUNT : 1u)
#define ESOP_DATAGRAM_STORAGE_COUNT (ESOP_DATAGRAM_COUNT ? ESOP_DATAGRAM_COUNT : 1u)
#define ESOP_SLAVE_COPY_STORAGE_COUNT (ESOP_SLAVE_COPY_COUNT ? ESOP_SLAVE_COPY_COUNT : 1u)
#define ESOP_SDO_INFORMATION_STORAGE_COUNT (ESOP_SDO_INFORMATION_COUNT ? ESOP_SDO_INFORMATION_COUNT : 1u)
#define ESOP_AXIS_STORAGE_COUNT (ESOP_AXIS_COUNT ? ESOP_AXIS_COUNT : 1u)

static const uint16_t esop_procbuf_abi_version = 7u;
static const uint32_t esop_procbuf_region_bytes = 7280u;
static const uint64_t esop_procbuf_layout_hash = UINT64_C(0x558a84ad9e20413b);

static const esop_slave_config_t esop_slaves[ESOP_SLAVE_STORAGE_COUNT] = {
  {"drive_00", 0u, UINT16_C(0x1001), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000001), 1u, 1u, 1u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0041), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_01", 1u, UINT16_C(0x1002), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000002), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0042), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_02", 2u, UINT16_C(0x1003), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000003), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0043), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_03", 3u, UINT16_C(0x1004), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000004), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0044), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_04", 4u, UINT16_C(0x1005), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000005), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0045), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_05", 5u, UINT16_C(0x1006), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000006), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0046), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_06", 6u, UINT16_C(0x1007), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000007), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0047), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_07", 7u, UINT16_C(0x1008), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000008), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0048), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_08", 8u, UINT16_C(0x1009), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000009), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0049), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_09", 9u, UINT16_C(0x100a), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x0000000a), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x004a), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_10", 10u, UINT16_C(0x100b), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x0000000b), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x004b), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_11", 11u, UINT16_C(0x100c), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x0000000c), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x004c), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_12", 12u, UINT16_C(0x100d), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x0000000d), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x004d), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_13", 13u, UINT16_C(0x100e), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x0000000e), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x004e), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_14", 14u, UINT16_C(0x100f), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x0000000f), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x004f), 1u, 0u, 1u, 0u, 0u, 0u},
  {"drive_15", 15u, UINT16_C(0x1010), 0u, 1u, UINT32_C(0x0000e500), UINT32_C(0x00004022), UINT32_C(0x00000001), UINT32_C(0x00000010), 1u, 1u, 0u, "DcSync", 1u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0300), INT16_C(1), 1u, UINT32_C(1000000), UINT32_C(0), INT32_C(0), UINT16_C(0x0300), UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 1u, UINT16_C(0x080d), UINT8_C(0x08), 1u, 4u, UINT16_C(0x000f), 3u, {1u, 2u, 3u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 1u, 1u, UINT16_C(2500), 1u, UINT16_C(100), 1u, 1u, UINT16_C(0x0050), 1u, 0u, 1u, 0u, 0u, 0u},
  {"io_block", 16u, UINT16_C(0x1011), 1u, 2u, UINT32_C(0x0000e500), UINT32_C(0x00007010), UINT32_C(0x00000001), UINT32_C(0x00000011), 1u, 0u, 0u, 0, 0u, UINT32_C(0), INT32_C(0), INT32_C(0), INT16_C(0), UINT16_C(0x0000), INT16_C(0), 0u, UINT32_C(0), UINT32_C(0), INT32_C(0), UINT16_C(0x0000), UINT16_C(0x1200), UINT16_C(32), UINT16_C(0x1300), UINT16_C(32), 0u, UINT8_C(0x26), 1u, UINT8_C(0x22), 0u, UINT16_C(0x0000), UINT8_C(0x00), 0u, 4u, UINT16_C(0x000f), 2u, {1u, 2u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u}, 0u, 0u, UINT16_C(0), 0u, UINT16_C(0), 1u, 1u, UINT16_C(0x0051), 0u, 0u, 0u, 0u, 0u, 0u},
};

static const esop_sdo_information_expectation_t esop_sdo_information_expectations[ESOP_SDO_INFORMATION_STORAGE_COUNT] = {
  {0},
};

static const esop_mailbox_status_mapping_t esop_mailbox_status_mappings[ESOP_SLAVE_STORAGE_COUNT] = {
  {0u, 1u, 0u, 3584u, 1u, 2u, UINT32_C(0x000011c0), 0u, 0u, UINT16_C(0x080d), 3u, 1u, 1u},
  {1u, 1u, 0u, 3585u, 1u, 2u, UINT32_C(0x000011c0), 1u, 1u, UINT16_C(0x080d), 3u, 1u, 1u},
  {2u, 1u, 0u, 3586u, 1u, 2u, UINT32_C(0x000011c0), 2u, 2u, UINT16_C(0x080d), 3u, 1u, 1u},
  {3u, 1u, 0u, 3587u, 1u, 2u, UINT32_C(0x000011c0), 3u, 3u, UINT16_C(0x080d), 3u, 1u, 1u},
  {4u, 1u, 0u, 3588u, 1u, 2u, UINT32_C(0x000011c0), 4u, 4u, UINT16_C(0x080d), 3u, 1u, 1u},
  {5u, 1u, 0u, 3589u, 1u, 2u, UINT32_C(0x000011c0), 5u, 5u, UINT16_C(0x080d), 3u, 1u, 1u},
  {6u, 1u, 0u, 3590u, 1u, 2u, UINT32_C(0x000011c0), 6u, 6u, UINT16_C(0x080d), 3u, 1u, 1u},
  {7u, 1u, 0u, 3591u, 1u, 2u, UINT32_C(0x000011c0), 7u, 7u, UINT16_C(0x080d), 3u, 1u, 1u},
  {8u, 1u, 0u, 3592u, 1u, 2u, UINT32_C(0x000011c1), 0u, 0u, UINT16_C(0x080d), 3u, 1u, 1u},
  {9u, 1u, 0u, 3593u, 1u, 2u, UINT32_C(0x000011c1), 1u, 1u, UINT16_C(0x080d), 3u, 1u, 1u},
  {10u, 1u, 0u, 3594u, 1u, 2u, UINT32_C(0x000011c1), 2u, 2u, UINT16_C(0x080d), 3u, 1u, 1u},
  {11u, 1u, 0u, 3595u, 1u, 2u, UINT32_C(0x000011c1), 3u, 3u, UINT16_C(0x080d), 3u, 1u, 1u},
  {12u, 1u, 0u, 3596u, 1u, 2u, UINT32_C(0x000011c1), 4u, 4u, UINT16_C(0x080d), 3u, 1u, 1u},
  {13u, 1u, 0u, 3597u, 1u, 2u, UINT32_C(0x000011c1), 5u, 5u, UINT16_C(0x080d), 3u, 1u, 1u},
  {14u, 1u, 0u, 3598u, 1u, 2u, UINT32_C(0x000011c1), 6u, 6u, UINT16_C(0x080d), 3u, 1u, 1u},
  {15u, 1u, 0u, 3599u, 1u, 2u, UINT32_C(0x000011c1), 7u, 7u, UINT16_C(0x080d), 3u, 1u, 1u},
  {16u, 0u, 0u, 0u, 0u, 0u, UINT32_C(0x00000000), 0u, 0u, UINT16_C(0x0000), 0u, 0u, 0u},
};

static const esop_domain_config_t esop_domains[ESOP_DOMAIN_STORAGE_COUNT] = {
  {"motion", 0u, UINT32_C(0x00001000), 0u, 450u, 208u, 242u, 1u, 0u, 32u},
  {"io", 1u, UINT32_C(0x00002000), 1024u, 9u, 7u, 2u, 4u, 0u, 2u},
};

static const esop_pdo_config_t esop_pdos[ESOP_PDO_STORAGE_COUNT] = {
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 0u, 16u, 0u},
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 16u, 8u, 1u},
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 24u, 32u, 1u},
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 56u, 32u, 1u},
  {0u, 0u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 88u, 16u, 1u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 104u, 16u, 0u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 120u, 8u, 1u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 128u, 32u, 1u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 160u, 32u, 1u},
  {0u, 1u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 192u, 16u, 1u},
  {0u, 2u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 208u, 16u, 0u},
  {0u, 2u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 224u, 8u, 1u},
  {0u, 2u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 232u, 32u, 1u},
  {0u, 2u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 264u, 32u, 1u},
  {0u, 2u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 296u, 16u, 1u},
  {0u, 3u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 312u, 16u, 0u},
  {0u, 3u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 328u, 8u, 1u},
  {0u, 3u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 336u, 32u, 1u},
  {0u, 3u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 368u, 32u, 1u},
  {0u, 3u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 400u, 16u, 1u},
  {0u, 4u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 416u, 16u, 0u},
  {0u, 4u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 432u, 8u, 1u},
  {0u, 4u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 440u, 32u, 1u},
  {0u, 4u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 472u, 32u, 1u},
  {0u, 4u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 504u, 16u, 1u},
  {0u, 5u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 520u, 16u, 0u},
  {0u, 5u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 536u, 8u, 1u},
  {0u, 5u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 544u, 32u, 1u},
  {0u, 5u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 576u, 32u, 1u},
  {0u, 5u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 608u, 16u, 1u},
  {0u, 6u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 624u, 16u, 0u},
  {0u, 6u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 640u, 8u, 1u},
  {0u, 6u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 648u, 32u, 1u},
  {0u, 6u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 680u, 32u, 1u},
  {0u, 6u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 712u, 16u, 1u},
  {0u, 7u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 728u, 16u, 0u},
  {0u, 7u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 744u, 8u, 1u},
  {0u, 7u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 752u, 32u, 1u},
  {0u, 7u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 784u, 32u, 1u},
  {0u, 7u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 816u, 16u, 1u},
  {0u, 8u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 832u, 16u, 0u},
  {0u, 8u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 848u, 8u, 1u},
  {0u, 8u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 856u, 32u, 1u},
  {0u, 8u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 888u, 32u, 1u},
  {0u, 8u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 920u, 16u, 1u},
  {0u, 9u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 936u, 16u, 0u},
  {0u, 9u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 952u, 8u, 1u},
  {0u, 9u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 960u, 32u, 1u},
  {0u, 9u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 992u, 32u, 1u},
  {0u, 9u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 1024u, 16u, 1u},
  {0u, 10u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 1040u, 16u, 0u},
  {0u, 10u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 1056u, 8u, 1u},
  {0u, 10u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 1064u, 32u, 1u},
  {0u, 10u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 1096u, 32u, 1u},
  {0u, 10u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 1128u, 16u, 1u},
  {0u, 11u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 1144u, 16u, 0u},
  {0u, 11u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 1160u, 8u, 1u},
  {0u, 11u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 1168u, 32u, 1u},
  {0u, 11u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 1200u, 32u, 1u},
  {0u, 11u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 1232u, 16u, 1u},
  {0u, 12u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 1248u, 16u, 0u},
  {0u, 12u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 1264u, 8u, 1u},
  {0u, 12u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 1272u, 32u, 1u},
  {0u, 12u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 1304u, 32u, 1u},
  {0u, 12u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 1336u, 16u, 1u},
  {0u, 13u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 1352u, 16u, 0u},
  {0u, 13u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 1368u, 8u, 1u},
  {0u, 13u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 1376u, 32u, 1u},
  {0u, 13u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 1408u, 32u, 1u},
  {0u, 13u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 1440u, 16u, 1u},
  {0u, 14u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 1456u, 16u, 0u},
  {0u, 14u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 1472u, 8u, 1u},
  {0u, 14u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 1480u, 32u, 1u},
  {0u, 14u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 1512u, 32u, 1u},
  {0u, 14u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 1544u, 16u, 1u},
  {0u, 15u, UINT16_C(0x1600), 2u, UINT16_C(0x6040), 0u, 0u, 1560u, 16u, 0u},
  {0u, 15u, UINT16_C(0x1600), 2u, UINT16_C(0x6060), 0u, 0u, 1576u, 8u, 1u},
  {0u, 15u, UINT16_C(0x1600), 2u, UINT16_C(0x607a), 0u, 0u, 1584u, 32u, 1u},
  {0u, 15u, UINT16_C(0x1600), 2u, UINT16_C(0x60ff), 0u, 0u, 1616u, 32u, 1u},
  {0u, 15u, UINT16_C(0x1600), 2u, UINT16_C(0x6071), 0u, 0u, 1648u, 16u, 1u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 1664u, 16u, 0u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 1680u, 8u, 1u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 1688u, 16u, 0u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 1704u, 32u, 1u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 1736u, 32u, 1u},
  {0u, 0u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 1768u, 16u, 1u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 1784u, 16u, 0u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 1800u, 8u, 1u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 1808u, 16u, 0u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 1824u, 32u, 1u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 1856u, 32u, 1u},
  {0u, 1u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 1888u, 16u, 1u},
  {0u, 2u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 1904u, 16u, 0u},
  {0u, 2u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 1920u, 8u, 1u},
  {0u, 2u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 1928u, 16u, 0u},
  {0u, 2u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 1944u, 32u, 1u},
  {0u, 2u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 1976u, 32u, 1u},
  {0u, 2u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2008u, 16u, 1u},
  {0u, 3u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2024u, 16u, 0u},
  {0u, 3u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2040u, 8u, 1u},
  {0u, 3u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2048u, 16u, 0u},
  {0u, 3u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2064u, 32u, 1u},
  {0u, 3u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2096u, 32u, 1u},
  {0u, 3u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2128u, 16u, 1u},
  {0u, 4u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2144u, 16u, 0u},
  {0u, 4u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2160u, 8u, 1u},
  {0u, 4u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2168u, 16u, 0u},
  {0u, 4u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2184u, 32u, 1u},
  {0u, 4u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2216u, 32u, 1u},
  {0u, 4u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2248u, 16u, 1u},
  {0u, 5u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2264u, 16u, 0u},
  {0u, 5u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2280u, 8u, 1u},
  {0u, 5u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2288u, 16u, 0u},
  {0u, 5u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2304u, 32u, 1u},
  {0u, 5u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2336u, 32u, 1u},
  {0u, 5u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2368u, 16u, 1u},
  {0u, 6u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2384u, 16u, 0u},
  {0u, 6u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2400u, 8u, 1u},
  {0u, 6u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2408u, 16u, 0u},
  {0u, 6u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2424u, 32u, 1u},
  {0u, 6u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2456u, 32u, 1u},
  {0u, 6u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2488u, 16u, 1u},
  {0u, 7u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2504u, 16u, 0u},
  {0u, 7u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2520u, 8u, 1u},
  {0u, 7u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2528u, 16u, 0u},
  {0u, 7u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2544u, 32u, 1u},
  {0u, 7u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2576u, 32u, 1u},
  {0u, 7u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2608u, 16u, 1u},
  {0u, 8u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2624u, 16u, 0u},
  {0u, 8u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2640u, 8u, 1u},
  {0u, 8u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2648u, 16u, 0u},
  {0u, 8u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2664u, 32u, 1u},
  {0u, 8u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2696u, 32u, 1u},
  {0u, 8u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2728u, 16u, 1u},
  {0u, 9u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2744u, 16u, 0u},
  {0u, 9u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2760u, 8u, 1u},
  {0u, 9u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2768u, 16u, 0u},
  {0u, 9u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2784u, 32u, 1u},
  {0u, 9u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2816u, 32u, 1u},
  {0u, 9u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2848u, 16u, 1u},
  {0u, 10u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2864u, 16u, 0u},
  {0u, 10u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 2880u, 8u, 1u},
  {0u, 10u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 2888u, 16u, 0u},
  {0u, 10u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 2904u, 32u, 1u},
  {0u, 10u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 2936u, 32u, 1u},
  {0u, 10u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 2968u, 16u, 1u},
  {0u, 11u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 2984u, 16u, 0u},
  {0u, 11u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 3000u, 8u, 1u},
  {0u, 11u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 3008u, 16u, 0u},
  {0u, 11u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 3024u, 32u, 1u},
  {0u, 11u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 3056u, 32u, 1u},
  {0u, 11u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 3088u, 16u, 1u},
  {0u, 12u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 3104u, 16u, 0u},
  {0u, 12u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 3120u, 8u, 1u},
  {0u, 12u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 3128u, 16u, 0u},
  {0u, 12u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 3144u, 32u, 1u},
  {0u, 12u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 3176u, 32u, 1u},
  {0u, 12u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 3208u, 16u, 1u},
  {0u, 13u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 3224u, 16u, 0u},
  {0u, 13u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 3240u, 8u, 1u},
  {0u, 13u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 3248u, 16u, 0u},
  {0u, 13u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 3264u, 32u, 1u},
  {0u, 13u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 3296u, 32u, 1u},
  {0u, 13u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 3328u, 16u, 1u},
  {0u, 14u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 3344u, 16u, 0u},
  {0u, 14u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 3360u, 8u, 1u},
  {0u, 14u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 3368u, 16u, 0u},
  {0u, 14u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 3384u, 32u, 1u},
  {0u, 14u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 3416u, 32u, 1u},
  {0u, 14u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 3448u, 16u, 1u},
  {0u, 15u, UINT16_C(0x1a00), 3u, UINT16_C(0x6041), 0u, 1u, 3464u, 16u, 0u},
  {0u, 15u, UINT16_C(0x1a00), 3u, UINT16_C(0x6061), 0u, 1u, 3480u, 8u, 1u},
  {0u, 15u, UINT16_C(0x1a00), 3u, UINT16_C(0x603f), 0u, 1u, 3488u, 16u, 0u},
  {0u, 15u, UINT16_C(0x1a00), 3u, UINT16_C(0x6064), 0u, 1u, 3504u, 32u, 1u},
  {0u, 15u, UINT16_C(0x1a00), 3u, UINT16_C(0x606c), 0u, 1u, 3536u, 32u, 1u},
  {0u, 15u, UINT16_C(0x1a00), 3u, UINT16_C(0x6077), 0u, 1u, 3568u, 16u, 1u},
  {1u, 16u, UINT16_C(0x1601), 2u, UINT16_C(0x7000), 1u, 0u, 0u, 16u, 0u},
  {1u, 16u, UINT16_C(0x1601), 2u, UINT16_C(0x7010), 1u, 0u, 16u, 32u, 1u},
  {1u, 16u, UINT16_C(0x1601), 2u, UINT16_C(0x7011), 1u, 0u, 48u, 8u, 0u},
  {1u, 16u, UINT16_C(0x1a01), 3u, UINT16_C(0x6000), 1u, 1u, 56u, 16u, 0u},
};

static const esop_datagram_config_t esop_datagrams[ESOP_DATAGRAM_STORAGE_COUNT] = {
  {0u, UINT8_C(0x0b), 0u, UINT32_C(0x00001000), 0u, 208u, 16u, 0u},
  {0u, UINT8_C(0x0a), 1u, UINT32_C(0x000010d0), 208u, 242u, 16u, 1u},
  {1u, UINT8_C(0x0b), 2u, UINT32_C(0x00002000), 1024u, 7u, 1u, 0u},
  {1u, UINT8_C(0x0a), 3u, UINT32_C(0x00002007), 1031u, 2u, 1u, 1u},
};

static const esop_slave_copy_config_t esop_slave_copies[ESOP_SLAVE_COPY_STORAGE_COUNT] = {
  {0},
};

static const esop_axis_config_t esop_axes[ESOP_AXIS_STORAGE_COUNT] = {
  {"joint_00", 0u, 0u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_01", 1u, 1u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_02", 2u, 2u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_03", 3u, 3u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_04", 4u, 4u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_05", 5u, 5u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_06", 6u, 6u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_07", 7u, 7u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_08", 8u, 8u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_09", 9u, 9u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_10", 10u, 10u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_11", 11u, 11u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_12", 12u, 12u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_13", 13u, 13u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_14", 14u, 14u, 8, 100000.00000000000000000, 1000.00000000000000000, 100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
  {"joint_15", 15u, 15u, 8, -100000.00000000000000000, -1000.00000000000000000, -100.00000000000000000, 0, -3.14159265358979312, 3.14159265358979312, 10.00000000000000000, 100.00000000000000000, 0.02000000000000000},
};

#endif
