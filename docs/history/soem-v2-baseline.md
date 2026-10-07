# SOEM v2.0.0 audit baseline

## Frozen upstream

- Repository: https://github.com/OpenEtherCATsociety/SOEM
- Release: https://github.com/OpenEtherCATsociety/SOEM/releases/tag/v2.0.0
- Tag commit: `304d1c05eab77dc0d426f1a5cf09c8cc7dc03713`
- Commit date: 2025-07-11
- License at the tag: GPLv3 or a commercial license

The inventory below is derived from the tag's public headers, source modules,
build file and shipped samples. It is a functional audit input, not permission
to copy implementation code.

## Public capability families

| Family | Upstream evidence | Functional surface |
| --- | --- | --- |
| Raw EtherCAT | `ec_base.h`, `ec_base.c` | AP/FP/BRD/BWR/LRD/LWR/LRW/ARMW/FRMW/LRWDC and compound datagrams |
| Main/NIC | `ec_main.h`, platform `nicdrv.c` | adapter enumeration, normal/redundant init, frame buffers, error queue |
| Discovery/state | `ec_config.h`, `ec_main.h` | scan/configure, AL read/write/check, slave/group state |
| SII/EEPROM | `ec_main.h` | byte/category access, SM/FMMU/PDO/string parsing, read/write, master/PDI ownership |
| Process data | `ec_main.h`, `ec_config.h` | mapping, groups, aligned/overlap image modes, send/receive and WKC |
| Mailbox | `ec_main.h` | send/receive, pool/queue, cyclic mailbox handling and ENI init commands |
| CoE | `ec_coe.h`, `ec_coe.c` | SDO read/write, Complete Access, RxPDO/TxPDO mailbox services, PDO-map discovery, OD/OE browsing |
| FoE | `ec_foe.h`, `ec_foe.c` | bounded file read/write and progress hook |
| SoE | `ec_soe.h`, `ec_soe.c` | IDN read/write and IDN mapping discovery |
| EoE | `ec_eoe.h`, `ec_eoe.c` | IP parameter get/set, fragmentation/reassembly, Ethernet send/receive and hook |
| DC | `ec_dc.h`, `ec_dc.c` | topology delay, clock configuration and SYNC0/SYNC1 |
| Redundancy | `ec_main.h`, platform `nicdrv.c` | dual-interface init, route detection, duplicate/reordered response handling |
| Recovery | `ec_config.h` | slave recovery and reconfiguration |
| ENI | `scripts/eniconv.py`, `cmake/AddENI.cmake`, `samples/eni_test` | ENI to static configuration and init commands |
| Build options | `CMakeLists.txt`, `ec_options.h.in` | build-time capacities, timeouts, retries and MAC values |
| Platforms | `osal/`, `oshw/`, `contrib/` | maintained Linux/Windows/RTK plus contributed embedded/RTOS ports |
| Tools/samples | `samples/` | basic master, slave info, EEPROM tool, firmware update, EoE and ENI workflows |
