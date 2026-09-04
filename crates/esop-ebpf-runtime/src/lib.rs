//! Linux eBPF loader and ring-buffer bridge for ESOP runtime evidence.
//!
//! This crate owns the Linux-only observation adapter. It loads a prebuilt
//! CO-RE ELF, attaches only the tracepoints it finds, and feeds validated fixed
//! records into [`esop_ebpf_agent::RuntimeAgent`]. It never receives an
//! EtherCAT port, PDO image, CiA 402 controlword, or motion permit.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::*;
