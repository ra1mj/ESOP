//! EtherCAT ESC register offsets and address helpers.
//!
//! EtherCAT datagrams carry a 16-bit address word followed by a 16-bit
//! register offset. Keeping the packing in one place prevents accidental
//! mixing of fixed-station and auto-increment addressing.

pub const ESC_TYPE: u16 = 0x0000;
pub const ESC_REVISION: u16 = 0x0001;
pub const ESC_BUILD: u16 = 0x0002;
pub const ESC_FMMU_COUNT: u16 = 0x0004;
pub const ESC_SYNC_MANAGER_COUNT: u16 = 0x0005;
pub const ESC_RAM_SIZE: u16 = 0x0006;
pub const ESC_PORT_DESCRIPTOR: u16 = 0x0007;

pub const ESC_DL_STATUS: u16 = 0x0110;
pub const ESC_STATION_ADDRESS: u16 = 0x0010;
pub const ESC_AL_STATUS: u16 = 0x0130;
pub const ESC_AL_STATUS_CODE: u16 = 0x0134;
pub const ESC_AL_CONTROL: u16 = 0x0120;
pub const ESC_EEPROM_CONTROL: u16 = 0x0502;
pub const ESC_EEPROM_ADDRESS: u16 = 0x0504;
pub const ESC_EEPROM_DATA: u16 = 0x0508;

pub const ESC_DC_TIME0: u16 = 0x0900;
pub const ESC_DC_TIME1: u16 = 0x0904;
pub const ESC_DC_TIME2: u16 = 0x0908;
pub const ESC_DC_TIME3: u16 = 0x090C;
pub const ESC_DC_SYSTEM_TIME: u16 = 0x0910;
pub const ESC_DC_SYSTEM_OFFSET: u16 = 0x0920;
pub const ESC_DC_SYSTEM_DELAY: u16 = 0x0928;
pub const ESC_DC_SYSTEM_DIFF: u16 = 0x092C;
pub const ESC_DC_CUC: u16 = 0x0980;
pub const ESC_DC_SYNC_ACTIVATION: u16 = 0x0981;
pub const ESC_DC_START0: u16 = 0x0990;
pub const ESC_DC_CYCLE0: u16 = 0x09A0;
pub const ESC_DC_CYCLE1: u16 = 0x09A4;

pub const BASIC_ESC_INFO_LEN: u16 = 9;
pub const AL_STATUS_WITH_CODE_LEN: u16 = 6;

/// Pack a fixed station address (FPRD/FPWR/FPRW) and ESC offset.
pub const fn fixed_address(station_address: u16, register: u16) -> u32 {
    ((station_address as u32) << 16) | register as u32
}

/// Pack an auto-increment position (APRD/APWR/APRW) and ESC offset.
///
/// Position zero addresses the first slave. Each subsequent position uses a
/// signed, decrementing auto-increment address: 0, -1, -2, ...
pub const fn auto_increment_address(position: u16, register: u16) -> u32 {
    let auto_increment = 0u16.wrapping_sub(position);
    ((auto_increment as u32) << 16) | register as u32
}

pub const fn station_from_address(address: u32) -> u16 {
    (address >> 16) as u16
}

pub const fn register_from_address(address: u32) -> u16 {
    address as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_helpers_pack_fixed_and_auto_increment_words() {
        assert_eq!(fixed_address(0x1000, ESC_AL_STATUS), 0x1000_0130);
        assert_eq!(auto_increment_address(0, ESC_TYPE), 0x0000_0000);
        assert_eq!(auto_increment_address(1, ESC_TYPE), 0xFFFF_0000);
        assert_eq!(auto_increment_address(2, ESC_TYPE), 0xFFFE_0000);
        assert_eq!(station_from_address(0x1234_0130), 0x1234);
        assert_eq!(register_from_address(0x1234_0130), ESC_AL_STATUS);
    }
}
