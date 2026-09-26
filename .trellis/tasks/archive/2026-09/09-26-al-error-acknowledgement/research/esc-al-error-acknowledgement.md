# ESC AL Error Acknowledgement Research

## Primary Source

Beckhoff Automation, *EtherCAT Slave Controller Hardware Data Sheet, Section I
- Technology*, version 2.3, 2017-02-21, chapter 10.1, page I-76.

Source:
https://download.beckhoff.com/download/document/io/ethercat-development-products/ethercat_esc_datasheet_sec1_technology_2i3.pdf

## Relevant Protocol Facts

- AL Control is at `0x0120:0x0121`; AL Status is at `0x0130:0x0131`; AL Status
  Code is at `0x0134:0x0135`.
- Device Emulation is indicated by ESC Configuration `0x0141[0]`.
- With Device Emulation enabled, the ESC copies AL Control directly into AL
  Status. The master must not set Error Indication Acknowledge for that device,
  because it would synthesize the Error Indication bit even when no error
  occurred.
- A normal slave reports Error Indication at `0x0130[4]` and a code at
  `0x0134:0x0135`. The master acknowledges it through `0x0120[4]`.

## Project Consequences

- Capability must be read explicitly before AL error handling policy is chosen.
- Device Emulation cannot share an unconditional acknowledgement path.
- The acknowledge command must retain the observed AL state in bits 3:0 and set
  only bit 4.
- Clearing Error Indication is not evidence that the failed transition is safe
  to retry. ESOP will retain a terminal startup fault and require explicit
  restart.
