use crate::SimError;

pub const MAX_OBJECT_BYTES: usize = 1024;
const MAILBOX: usize = 128;
const SMALL_OBJECTS: usize = 64;
const RAM_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveKind {
    Drive,
    Io,
    Generic,
}

/// EEPROM is a device ROM supplied independently of master configuration.
#[derive(Clone, Copy, Debug)]
pub struct SlaveConfig {
    pub kind: SlaveKind,
    pub sii: &'static [u16],
    pub serial: u32,
    pub requesting_id: u16,
    pub dc: bool,
}
impl SlaveConfig {
    pub(crate) fn validate(self) -> Result<(), SimError> {
        if self.sii.len() < 0x40
            || self.sii[0x1d] as usize > MAILBOX
            || self.sii[0x1f] as usize > MAILBOX
            || self.sii[0x1d] < 16
            || self.sii[0x1f] < 16
        {
            return Err(SimError::InvalidConfiguration);
        }
        let send = self.sii[0x1c] as u32;
        let send_end = send + self.sii[0x1d] as u32;
        let receive = self.sii[0x1e] as u32;
        let receive_end = receive + self.sii[0x1f] as u32;
        if send == 0
            || receive == 0
            || send_end > 65536
            || receive_end > 65536
            || (send < receive_end && receive < send_end)
        {
            return Err(SimError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveSnapshot {
    pub online: bool,
    pub station_address: u16,
    pub al_state: u8,
    pub al_error: u16,
    pub mailbox_send_full: bool,
    pub mailbox_receive_full: bool,
    pub mailbox_repeats: u64,
    pub statusword: u16,
    pub mode: i8,
    pub position: i32,
    pub velocity: i32,
    pub torque: i16,
    pub digital_inputs: u16,
    pub digital_outputs: u16,
    pub sync_activation: u8,
}

#[derive(Clone, Copy)]
struct Object {
    index: u16,
    sub: u8,
    len: u8,
    data: [u8; 8],
}

#[derive(Clone, Copy)]
struct LargeObject {
    index: u16,
    sub: u8,
    len: usize,
    data: [u8; MAX_OBJECT_BYTES],
}
impl LargeObject {
    const EMPTY: Self = Self {
        index: 0,
        sub: 0,
        len: 0,
        data: [0; MAX_OBJECT_BYTES],
    };
}
impl Object {
    const EMPTY: Self = Self {
        index: 0,
        sub: 0,
        len: 0,
        data: [0; 8],
    };
}

#[derive(Clone, Copy)]
struct Transfer {
    active: bool,
    upload: bool,
    index: u16,
    sub: u8,
    len: usize,
    offset: usize,
    toggle: bool,
    data: [u8; MAX_OBJECT_BYTES],
}
impl Transfer {
    const EMPTY: Self = Self {
        active: false,
        upload: false,
        index: 0,
        sub: 0,
        len: 0,
        offset: 0,
        toggle: false,
        data: [0; MAX_OBJECT_BYTES],
    };
}

pub(crate) struct Esc {
    config: SlaveConfig,
    low: [u8; 0x200],
    watchdog: [u8; 0x50],
    eeprom: [u8; 16],
    fmmu: [[u8; 16]; 16],
    sm: [[u8; 8]; 16],
    dc: [u8; 0xb0],
    ram: [[u8; RAM_BYTES]; 16],
    send: [u8; MAILBOX],
    receive: [u8; MAILBOX],
    previous: [u8; MAILBOX],
    send_full: bool,
    receive_full: bool,
    previous_valid: bool,
    objects: [Object; SMALL_OBJECTS],
    large: [LargeObject; 8],
    transfer: Transfer,
    pub online: bool,
    pub dc_fault: i64,
    pub repeat_ack: bool,
    pub counter: u8,
    repeats: u64,
    statusword: u16,
    mode: i8,
    position: i32,
    velocity: i32,
    torque: i16,
    drive_error: u16,
    drive_fault_input: u16,
    digital_inputs: u16,
    digital_outputs: u16,
    last_pdo_ns: u64,
}

impl Esc {
    pub(crate) fn new(config: SlaveConfig) -> Self {
        let mut value = Self {
            config,
            low: [0; 0x200],
            watchdog: [0; 0x50],
            eeprom: [0; 16],
            fmmu: [[0; 16]; 16],
            sm: [[0; 8]; 16],
            dc: [0; 0xb0],
            ram: [[0; RAM_BYTES]; 16],
            send: [0; MAILBOX],
            receive: [0; MAILBOX],
            previous: [0; MAILBOX],
            send_full: false,
            receive_full: false,
            previous_valid: false,
            objects: [Object::EMPTY; SMALL_OBJECTS],
            large: [LargeObject::EMPTY; 8],
            transfer: Transfer::EMPTY,
            online: true,
            dc_fault: 0,
            repeat_ack: true,
            counter: 1,
            repeats: 0,
            statusword: 0x40,
            mode: 0,
            position: 0,
            velocity: 0,
            torque: 0,
            drive_error: 0,
            drive_fault_input: 0,
            digital_inputs: 0,
            digital_outputs: 0,
            last_pdo_ns: 0,
        };
        value.low[..12].copy_from_slice(&[
            0x11,
            1,
            1,
            0,
            16,
            16,
            16,
            0x11,
            if config.dc { 0x0d } else { 1 },
            0,
            0,
            0,
        ]);
        value.low[0x110..0x112].copy_from_slice(&0x0a30u16.to_le_bytes());
        value.low[0x130] = 1;
        for (i, (address, capacity, control)) in [
            (config.sii[0x1c], config.sii[0x1d], 0x26),
            (config.sii[0x1e], config.sii[0x1f], 0x22),
        ]
        .into_iter()
        .enumerate()
        {
            value.sm[i][..2].copy_from_slice(&address.to_le_bytes());
            value.sm[i][2..4].copy_from_slice(&capacity.to_le_bytes());
            value.sm[i][4] = control;
            value.sm[i][6] = 1;
        }
        // Application-side initial OD, independent of any requested transfer.
        for (index, data) in [(0x1000, 0x0002_0192u32), (0x1018, 0), (0x2000, 0)] {
            let _ = value.set_object(index, 0, &data.to_le_bytes());
        }
        let _ = value.set_object(0x1018, 0, &[4]);
        for (sub, word) in [(1, 8), (2, 10), (3, 12)] {
            let data =
                (config.sii[word] as u32 | ((config.sii[word + 1] as u32) << 16)).to_le_bytes();
            let _ = value.set_object(0x1018, sub, &data);
        }
        let _ = value.set_object(0x1018, 4, &config.serial.to_le_bytes());
        value.initialize_pdo_objects();
        value
    }

    fn initialize_pdo_objects(&mut self) {
        let mut cursor = 0x40;
        for _ in 0..128 {
            if cursor + 2 > self.config.sii.len() {
                break;
            }
            let kind = self.config.sii[cursor];
            let words = self.config.sii[cursor + 1] as usize;
            if kind == 0xffff || cursor + 2 + words > self.config.sii.len() {
                break;
            }
            if matches!(kind, 50 | 51) && words >= 4 {
                let start = cursor + 2;
                let index = self.config.sii[start];
                let count = self.config.sii[start + 1] as u8;
                if 4 + count as usize * 4 <= words {
                    let _ = self.set_object(index, 0, &[count]);
                    for entry in 0..count as usize {
                        let e = start + 4 + entry * 4;
                        let object = self.config.sii[e];
                        let sub = self.config.sii[e + 1] as u8;
                        let bits = self.config.sii[e + 2] as u8;
                        let packed = (object as u32) << 16 | (sub as u32) << 8 | bits as u32;
                        let _ = self.set_object(index, (entry + 1) as u8, &packed.to_le_bytes());
                    }
                    let assignment = if kind == 51 { 0x1c12 } else { 0x1c13 };
                    let n = self
                        .object(assignment, 0)
                        .and_then(|b| b.first())
                        .copied()
                        .unwrap_or(0);
                    let _ = self.set_object(assignment, n + 1, &index.to_le_bytes());
                    let _ = self.set_object(assignment, 0, &[n + 1]);
                }
            }
            cursor += 2 + words;
        }
    }

    fn pdo_field(&self, sm: usize, index: u16, sub: u8) -> Option<(usize, usize)> {
        let assignment = if sm == 2 { 0x1c12 } else { 0x1c13 };
        let count = *self.object(assignment, 0)?.first()? as usize;
        let mut bit = 0;
        for group in 1..=count {
            let bytes = self.object(assignment, group as u8)?;
            let mapping = u16::from_le_bytes([*bytes.first()?, *bytes.get(1)?]);
            let entries = *self.object(mapping, 0)?.first()? as usize;
            for entry in 1..=entries {
                let bytes = self.object(mapping, entry as u8)?;
                if bytes.len() != 4 {
                    return None;
                }
                let packed = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let bits = (packed & 255) as usize;
                if (packed >> 16) as u16 == index && ((packed >> 8) & 255) as u8 == sub {
                    return Some((bit, bits));
                }
                bit += bits;
            }
        }
        None
    }
    fn application_read(&self, index: u16, sub: u8) -> u64 {
        let Some((offset, bits)) = self.pdo_field(2, index, sub) else {
            return 0;
        };
        if bits > 64 || offset + bits > RAM_BYTES * 8 {
            return 0;
        }
        let mut value = 0;
        for bit in 0..bits {
            value |=
                (((self.ram[2][(offset + bit) / 8] >> ((offset + bit) % 8)) & 1) as u64) << bit;
        }
        value
    }
    fn application_write(&mut self, index: u16, sub: u8, value: u64) {
        let Some((offset, bits)) = self.pdo_field(3, index, sub) else {
            return;
        };
        if bits > 64 || offset + bits > RAM_BYTES * 8 {
            return;
        }
        for bit in 0..bits {
            let destination = &mut self.ram[3][(offset + bit) / 8];
            let mask = 1 << ((offset + bit) % 8);
            *destination =
                (*destination & !mask) | ((((value >> bit) & 1) as u8) << ((offset + bit) % 8));
        }
    }

    pub(crate) fn station(&self) -> u16 {
        u16::from_le_bytes([self.low[0x10], self.low[0x11]])
    }
    pub(crate) fn set_downstream_link(&mut self, connected: bool) {
        let dl = if connected { 0x5a30u16 } else { 0x5610u16 };
        self.low[0x110..0x112].copy_from_slice(&dl.to_le_bytes());
    }
    pub(crate) fn snapshot(&self) -> SlaveSnapshot {
        SlaveSnapshot {
            online: self.online,
            station_address: self.station(),
            al_state: self.low[0x130] & 15,
            al_error: u16::from_le_bytes([self.low[0x134], self.low[0x135]]),
            mailbox_send_full: self.send_full,
            mailbox_receive_full: self.receive_full,
            mailbox_repeats: self.repeats,
            statusword: self.statusword,
            mode: self.mode,
            position: self.position,
            velocity: self.velocity,
            torque: self.torque,
            digital_inputs: self.digital_inputs,
            digital_outputs: self.digital_outputs,
            sync_activation: self.dc[0x81],
        }
    }
    pub(crate) fn al_fault(&mut self, code: u16) {
        self.low[0x134..0x136].copy_from_slice(&code.to_le_bytes());
        if code != 0 {
            self.low[0x130] |= 0x10;
        }
    }
    pub(crate) fn drive_fault(&mut self, code: u16) {
        self.drive_fault_input = code;
        if code != 0 {
            self.drive_error = code;
            self.statusword = 8;
        }
    }
    fn sm_start(&self, index: usize) -> u16 {
        u16::from_le_bytes([self.sm[index][0], self.sm[index][1]])
    }
    fn sm_len(&self, index: usize) -> usize {
        u16::from_le_bytes([self.sm[index][2], self.sm[index][3]]) as usize
    }
    fn sm_enabled(&self, index: usize) -> bool {
        self.sm[index][6] & 1 != 0
            && self.sm[index][7] & 1 == 0
            && self.sm_len(index) > 0
            && self.sm_len(index) <= RAM_BYTES
    }

    pub(crate) fn set_object(&mut self, index: u16, sub: u8, data: &[u8]) -> Result<(), SimError> {
        if data.is_empty() || data.len() > MAX_OBJECT_BYTES {
            return Err(SimError::ObjectCapacity);
        }
        if let Some(slot) = self
            .large
            .iter_mut()
            .find(|o| o.len != 0 && o.index == index && o.sub == sub)
        {
            slot.len = data.len();
            slot.data[..data.len()].copy_from_slice(data);
            return Ok(());
        }
        if data.len() > 8 {
            let position = self
                .large
                .iter()
                .position(|o| o.len != 0 && o.index == index && o.sub == sub)
                .or_else(|| self.large.iter().position(|o| o.len == 0))
                .ok_or(SimError::ObjectCapacity)?;
            let slot = &mut self.large[position];
            slot.index = index;
            slot.sub = sub;
            slot.len = data.len();
            slot.data[..data.len()].copy_from_slice(data);
        } else {
            let slot = self
                .objects
                .iter_mut()
                .find(|o| o.index == index && o.sub == sub);
            if let Some(slot) = slot {
                slot.len = data.len() as u8;
                slot.data[..data.len()].copy_from_slice(data);
                return Ok(());
            }
            let slot = self
                .objects
                .iter_mut()
                .find(|o| o.len == 0)
                .ok_or(SimError::ObjectCapacity)?;
            *slot = Object::EMPTY;
            slot.index = index;
            slot.sub = sub;
            slot.len = data.len() as u8;
            slot.data[..data.len()].copy_from_slice(data);
        }
        Ok(())
    }
    fn object(&self, index: u16, sub: u8) -> Option<&[u8]> {
        if let Some(object) = self
            .large
            .iter()
            .find(|o| o.len != 0 && o.index == index && o.sub == sub)
        {
            return Some(&object.data[..object.len]);
        }
        self.objects
            .iter()
            .find(|o| o.len != 0 && o.index == index && o.sub == sub)
            .map(|o| &o.data[..o.len as usize])
    }

    fn raw_read(&self, address: u16, now: u64) -> u8 {
        let a = address as usize;
        match a {
            0..=0x1ff => self.low[a],
            0x400..=0x44f => self.watchdog[a - 0x400],
            0x500..=0x50f => self.eeprom[a - 0x500],
            0x600..=0x6ff => self.fmmu[(a - 0x600) / 16][a % 16],
            0x800..=0x87f => {
                let sm = (a - 0x800) / 8;
                let offset = a % 8;
                if offset == 5 && sm < 2 {
                    if (sm == 0 && self.send_full) || (sm == 1 && self.receive_full) {
                        8
                    } else {
                        0
                    }
                } else {
                    self.sm[sm][offset]
                }
            }
            0x900..=0x9af => {
                if (0x910..0x918).contains(&a) {
                    let offset =
                        i64::from_le_bytes(self.dc[0x20..0x28].try_into().unwrap_or([0; 8]));
                    now.wrapping_add_signed(offset)
                        .wrapping_add_signed(self.dc_fault)
                        .to_le_bytes()[a - 0x910]
                } else if a < 0x910 {
                    let port = (a - 0x900) / 4;
                    ((port as u32 * 100) + 50).to_le_bytes()[a % 4]
                } else if (0x92c..0x930).contains(&a) {
                    (self.dc_fault as i32).to_le_bytes()[a - 0x92c]
                } else {
                    self.dc[a - 0x900]
                }
            }
            _ => 0,
        }
    }
    fn raw_write(&mut self, address: u16, value: u8) {
        let a = address as usize;
        match a {
            0x10..=0x13 | 0x120..=0x121 => self.low[a] = value,
            0x400..=0x44f => self.watchdog[a - 0x400] = value,
            0x500..=0x507 => self.eeprom[a - 0x500] = value,
            0x600..=0x6ff => self.fmmu[(a - 0x600) / 16][a % 16] = value,
            0x800..=0x87f => {
                let sm = (a - 0x800) / 8;
                let offset = a % 8;
                if offset != 5 {
                    self.sm[sm][offset] = if offset == 6 { value & 0xc3 } else { value };
                }
                if sm == 1 && offset == 6 && self.repeat_ack && value & 2 != self.sm[sm][7] & 2 {
                    self.sm[sm][7] = (self.sm[sm][7] & !2) | (value & 2);
                    if self.previous_valid {
                        self.receive = self.previous;
                        self.receive_full = true;
                        self.repeats = self.repeats.saturating_add(1);
                    }
                }
            }
            0x918..=0x9af => self.dc[a - 0x900] = value,
            _ => {}
        }
    }

    pub(crate) fn read(&mut self, address: u16, data: &mut [u8], now: u64) -> bool {
        if address as usize + data.len() > 65536
            || (!self.config.dc && (0x900..0x9b0).contains(&address))
        {
            return false;
        }
        let recv = self.sm_start(1) as usize;
        if address as usize >= recv && (address as usize) < recv + self.sm_len(1) {
            let offset = address as usize - recv;
            if !self.sm_enabled(1) || offset + data.len() > self.sm_len(1) || !self.receive_full {
                return false;
            }
            data.copy_from_slice(&self.receive[offset..offset + data.len()]);
            if offset + data.len() == self.sm_len(1) {
                self.receive_full = false;
            }
            return true;
        }
        for i in 2..16 {
            let start = self.sm_start(i) as usize;
            if address as usize >= start
                && (address as usize) < start + self.sm_len(i)
                && self.sm_len(i) <= RAM_BYTES
            {
                let offset = address as usize - start;
                if !self.sm_enabled(i) || offset + data.len() > self.sm_len(i) {
                    return false;
                }
                data.copy_from_slice(&self.ram[i][offset..offset + data.len()]);
                return true;
            }
        }
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.raw_read(address + i as u16, now);
        }
        true
    }
    pub(crate) fn write(&mut self, address: u16, data: &[u8], now: u64) -> bool {
        if address as usize + data.len() > 65536
            || (!self.config.dc && (0x900..0x9b0).contains(&address))
        {
            return false;
        }
        let send = self.sm_start(0) as usize;
        if address as usize >= send && (address as usize) < send + self.sm_len(0) {
            let offset = address as usize - send;
            if !self.sm_enabled(0) || self.send_full || offset + data.len() > self.sm_len(0) {
                return false;
            }
            self.send[offset..offset + data.len()].copy_from_slice(data);
            if offset + data.len() == self.sm_len(0) {
                self.send_full = true;
            }
            return true;
        }
        for i in 2..16 {
            let start = self.sm_start(i) as usize;
            if address as usize >= start
                && (address as usize) < start + self.sm_len(i)
                && self.sm_len(i) <= RAM_BYTES
            {
                let offset = address as usize - start;
                if !self.sm_enabled(i) || offset + data.len() > self.sm_len(i) {
                    return false;
                }
                self.ram[i][offset..offset + data.len()].copy_from_slice(data);
                self.last_pdo_ns = now;
                return true;
            }
        }
        if address == 0x910 && matches!(data.len(), 4 | 8) {
            let mut bytes = [0; 8];
            bytes[..data.len()].copy_from_slice(data);
            let offset = u64::from_le_bytes(bytes)
                .wrapping_sub(now)
                .wrapping_sub(self.dc_fault as u64);
            self.dc[0x20..0x28].copy_from_slice(&offset.to_le_bytes());
            return true;
        }
        for (i, byte) in data.iter().enumerate() {
            self.raw_write(address + i as u16, *byte);
        }
        if address <= 0x120 && address as usize + data.len() >= 0x122 {
            self.change_state(now);
        }
        if address <= 0x502 && address as usize + data.len() >= 0x504 && self.eeprom[3] & 1 != 0 {
            let word = u32::from_le_bytes([
                self.eeprom[4],
                self.eeprom[5],
                self.eeprom[6],
                self.eeprom[7],
            ]) as usize;
            for i in 0..4 {
                let location = word + i;
                let value = if location == 0xe {
                    self.config.serial as u16
                } else if location == 0xf {
                    (self.config.serial >> 16) as u16
                } else {
                    self.config.sii.get(location).copied().unwrap_or(0xffff)
                };
                self.eeprom[8 + i * 2..10 + i * 2].copy_from_slice(&value.to_le_bytes());
            }
            self.eeprom[2] = 0;
            self.eeprom[3] = 0;
        }
        true
    }
    fn change_state(&mut self, now: u64) {
        let requested = self.low[0x120];
        let state = requested & 15;
        if requested & 0x10 != 0 {
            self.low[0x130] &= !0x10;
            self.low[0x134] = 0;
            self.low[0x135] = 0;
        }
        if self.low[0x130] & 0x10 != 0 {
            return;
        }
        if !matches!(state, 1 | 2 | 3 | 4 | 8) {
            self.al_fault(0x12);
            return;
        }
        let current = self.low[0x130] & 15;
        let legal = match current {
            1 => matches!(state, 1..=3),
            2 => matches!(state, 1 | 2 | 4),
            3 => matches!(state, 1 | 3),
            4 | 8 => matches!(state, 1 | 2 | 4 | 8),
            _ => false,
        };
        if !legal {
            self.al_fault(0x11);
            return;
        }
        // A process-data transition requires actual enabled SMs and FMMUs;
        // INIT/PREOP remain possible before process configuration.
        if matches!(state, 4 | 8)
            && (!self.sm_enabled(3) || !self.fmmu.iter().any(|f| f[12] & 1 != 0))
        {
            self.al_fault(0x1d);
            return;
        }
        self.low[0x130] = state | (requested & 0x20);
        if state == 8 {
            self.last_pdo_ns = now;
        }
        if requested & 0x20 != 0 {
            self.low[0x134..0x136].copy_from_slice(&self.config.requesting_id.to_le_bytes());
        } else {
            self.low[0x134] = 0;
            self.low[0x135] = 0;
        }
    }

    pub(crate) fn logical(
        &mut self,
        command: u8,
        address: u32,
        data: &mut [u8],
        original: &[u8],
        now: u64,
    ) -> u16 {
        let mut did_read = false;
        let mut did_write = false;
        let end = address as u64 * 8 + data.len() as u64 * 8;
        for f in self.fmmu {
            if f[12] & 1 == 0 {
                continue;
            }
            let logical = u32::from_le_bytes([f[0], f[1], f[2], f[3]]) as u64 * 8 + f[6] as u64;
            let len = u16::from_le_bytes([f[4], f[5]]) as u64;
            if len == 0 {
                continue;
            }
            let map_end = (logical / 8 + len - 1) * 8 + f[7] as u64 + 1;
            let start = logical.max(address as u64 * 8);
            let stop = map_end.min(end);
            if start >= stop {
                continue;
            }
            let physical = u16::from_le_bytes([f[8], f[9]]) as u64 * 8 + f[10] as u64;
            for bit in start..stop {
                let p = physical + bit - logical;
                let d = bit - address as u64 * 8;
                if matches!(command, 10 | 12)
                    && f[11] & 1 != 0
                    && matches!(self.low[0x130] & 15, 4 | 8)
                {
                    if let Some(value) = self.process_read(p / 8, now) {
                        let mask = 1 << (d % 8);
                        data[d as usize / 8] =
                            (data[d as usize / 8] & !mask) | (((value >> (p % 8)) & 1) << (d % 8));
                        did_read = true;
                    }
                }
                if matches!(command, 11 | 12)
                    && f[11] & 2 != 0
                    && (self.low[0x130] & 15) == 8
                    && self.process_write(
                        p / 8,
                        (p % 8) as u8,
                        (original[d as usize / 8] >> (d % 8)) & 1,
                    )
                {
                    did_write = true;
                    self.last_pdo_ns = now;
                }
            }
        }
        match command {
            12 => u16::from(did_read) + 2 * u16::from(did_write),
            _ => u16::from(did_read || did_write),
        }
    }
    fn process_read(&self, address: u64, now: u64) -> Option<u8> {
        // Status FMMUs do not require process RAM or an input SM.
        if (0x800..0x880).contains(&address) {
            return Some(self.raw_read(address as u16, now));
        }
        for i in 2..16 {
            let start = self.sm_start(i) as u64;
            if self.sm_enabled(i)
                && address >= start
                && address < start + self.sm_len(i) as u64
                && self.sm_len(i) <= RAM_BYTES
            {
                return Some(self.ram[i][(address - start) as usize]);
            }
        }
        None
    }
    fn process_write(&mut self, address: u64, bit: u8, value: u8) -> bool {
        for i in 2..16 {
            let start = self.sm_start(i) as u64;
            if self.sm_enabled(i)
                && address >= start
                && address < start + self.sm_len(i) as u64
                && self.sm_len(i) <= RAM_BYTES
            {
                let byte = &mut self.ram[i][(address - start) as usize];
                *byte = (*byte & !(1 << bit)) | (value << bit);
                return true;
            }
        }
        false
    }

    pub(crate) fn tick(&mut self, now: u64, elapsed: u64) {
        if !self.online {
            return;
        }
        if self.send_full && !self.receive_full && self.sm_enabled(0) && self.sm_enabled(1) {
            self.process_mailbox();
            self.send_full = false;
        }
        if self.low[0x130] & 15 != 8 {
            self.velocity = 0;
        }
        match self.config.kind {
            SlaveKind::Drive => self.tick_drive(elapsed),
            SlaveKind::Io => {
                self.digital_outputs = self.application_read(0x7000, 1) as u16;
                self.digital_inputs = self.digital_outputs;
                self.application_write(0x6000, 1, self.digital_inputs as u64);
            }
            SlaveKind::Generic => {}
        }
        if self.config.kind == SlaveKind::Drive && self.pdo_field(2, 0x7000, 1).is_some() {
            self.digital_outputs = self.application_read(0x7000, 1) as u16;
            self.digital_inputs = self.digital_outputs;
            self.application_write(0x6000, 1, self.digital_inputs as u64);
        }
        let intervals = u16::from_le_bytes([self.watchdog[0x20], self.watchdog[0x21]]) as u64;
        let divider = u16::from_le_bytes([self.watchdog[0], self.watchdog[1]]) as u64;
        let timeout = (divider + 2).saturating_mul(40).saturating_mul(intervals);
        if self.low[0x130] & 15 == 8
            && intervals != 0
            && now.saturating_sub(self.last_pdo_ns) > timeout
        {
            self.low[0x130] = 4;
            self.al_fault(0x1b);
        }
    }
    fn tick_drive(&mut self, elapsed: u64) {
        let control = self.application_read(0x6040, 0) as u16;
        self.mode = self.application_read(0x6060, 0) as i8;
        if self.drive_error != 0 {
            if control & 0x80 != 0 && self.drive_fault_input == 0 {
                self.drive_error = 0;
                self.statusword = 0x40;
            } else {
                self.statusword = 8;
            }
        } else if control & 2 == 0 {
            self.statusword = 0x40;
        } else if control & 0x80 == 0 {
            // Advance one legal PDS edge per application tick. An Enable
            // Operation command cannot jump from Switch On Disabled to OP.
            let command = control & 15;
            self.statusword = match self.statusword & 0x6f {
                0x40 => {
                    if command & 7 == 6 {
                        0x21
                    } else {
                        0x40
                    }
                }
                0x21 => {
                    if command & 7 == 7 {
                        0x23
                    } else if command & 4 == 0 {
                        0x40
                    } else {
                        0x21
                    }
                }
                0x23 => {
                    if command == 15 {
                        0x27
                    } else if command & 7 == 6 {
                        0x21
                    } else if command & 4 == 0 {
                        0x40
                    } else {
                        0x23
                    }
                }
                0x27 => {
                    if command & 4 == 0 {
                        7
                    } else if command == 7 {
                        0x23
                    } else if command & 7 == 6 {
                        0x21
                    } else {
                        0x27
                    }
                }
                7 => {
                    if command == 15 {
                        0x27
                    } else if command & 7 == 6 {
                        0x21
                    } else {
                        7
                    }
                }
                _ => 0x40,
            };
        }
        if self.low[0x130] & 15 == 8 && self.statusword == 0x27 {
            let target = match self.mode {
                9 => self.application_read(0x60ff, 0) as i32,
                10 => self.application_read(0x6071, 0) as i16 as i32,
                _ => self.application_read(0x607a, 0) as i32,
            };
            match self.mode {
                8 => {
                    let previous = self.position;
                    self.position = target;
                    self.velocity = if elapsed == 0 {
                        0
                    } else {
                        ((self.position as i64 - previous as i64) * 1_000_000_000
                            / (elapsed.min(i64::MAX as u64) as i64))
                            .clamp(i32::MIN as i64, i32::MAX as i64) as i32
                    };
                }
                9 => {
                    self.velocity = target;
                    self.position = self.position.saturating_add(
                        (target as i64 * elapsed.min(1_000_000_000) as i64 / 1_000_000_000) as i32,
                    );
                }
                10 => {
                    self.torque = target as i16;
                    self.velocity = 0;
                }
                _ => {
                    self.velocity = 0;
                }
            }
        } else {
            self.velocity = 0;
            self.torque = 0;
        }
        self.ram[3].fill(0);
        for (index, value) in [
            (0x6041, self.statusword as u64),
            (0x6061, self.mode as u8 as u64),
            (0x603f, self.drive_error as u64),
            (0x6064, self.position as u32 as u64),
            (0x606c, self.velocity as u32 as u64),
            (0x6077, self.torque as u16 as u64),
        ] {
            self.application_write(index, 0, value);
        }
    }

    fn publish_mailbox(&mut self, payload: &[u8], protocol: u8) {
        self.receive.fill(0);
        self.receive[..2].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        self.receive[5] = protocol | ((self.counter & 7) << 4);
        self.counter = if self.counter >= 7 {
            1
        } else {
            self.counter + 1
        };
        self.receive[6..6 + payload.len()].copy_from_slice(payload);
        self.receive_full = true;
        self.previous = self.receive;
        self.previous_valid = true;
    }
    pub(crate) fn emergency(
        &mut self,
        code: u16,
        register: u8,
        data: [u8; 5],
    ) -> Result<(), SimError> {
        if self.receive_full {
            return Err(SimError::MailboxFull);
        }
        let mut payload = [0; 10];
        payload[1] = 0x10;
        payload[2..4].copy_from_slice(&code.to_le_bytes());
        payload[4] = register;
        payload[5..].copy_from_slice(&data);
        self.publish_mailbox(&payload, 3);
        Ok(())
    }
    fn abort(&mut self, index: u16, sub: u8, code: u32) {
        let mut p = [0; 10];
        p[1] = 0x30;
        p[2] = 0x80;
        p[3..5].copy_from_slice(&index.to_le_bytes());
        p[5] = sub;
        p[6..10].copy_from_slice(&code.to_le_bytes());
        self.transfer.active = false;
        self.publish_mailbox(&p, 3);
    }
    fn process_mailbox(&mut self) {
        let size = u16::from_le_bytes([self.send[0], self.send[1]]) as usize;
        if size < 3 || size + 6 > self.sm_len(0) || self.send[5] & 15 != 3 {
            self.publish_mailbox(&[1, 0, 2, 0], 0);
            return;
        }
        if self.send[7] >> 4 == 8 {
            self.information(size);
            return;
        }
        if self.send[7] >> 4 != 2 {
            self.publish_mailbox(&[1, 0, 2, 0], 0);
            return;
        }
        let request = self.send;
        let p = &request[6..6 + size];
        let command = p[2];
        if command == 0x80 {
            self.transfer.active = false;
            return;
        }
        if self.transfer.active && self.transfer.upload && command & 0xef == 0x60 {
            self.upload_segment(command);
            return;
        }
        if self.transfer.active && !self.transfer.upload && command & 0xe0 == 0 {
            self.download_segment(command, p);
            return;
        }
        if p.len() < 6 {
            self.abort(0, 0, 0x0504_0001);
            return;
        }
        let index = u16::from_le_bytes([p[3], p[4]]);
        let sub = p[5];
        let mut reply = [0; MAILBOX];
        reply[1] = 0x30;
        reply[3..6].copy_from_slice(&p[3..6]);
        match command & 0xe0 {
            0x40 => {
                let Some(object) = self.object(index, sub) else {
                    self.abort(index, sub, 0x0602_0000);
                    return;
                };
                let len = object.len();
                let mut bytes = [0; MAX_OBJECT_BYTES];
                bytes[..len].copy_from_slice(object);
                if len <= 4 {
                    reply[2] = 0x43 | ((4 - len) as u8) << 2 | (command & 0x10);
                    reply[6..6 + len].copy_from_slice(&bytes[..len]);
                    self.publish_mailbox(&reply[..10], 3);
                } else {
                    reply[2] = 0x41 | (command & 0x10);
                    reply[6..10].copy_from_slice(&(len as u32).to_le_bytes());
                    let chunk = len.min(self.sm_len(1).saturating_sub(16));
                    reply[10..10 + chunk].copy_from_slice(&bytes[..chunk]);
                    self.transfer = Transfer {
                        active: chunk < len,
                        upload: true,
                        index,
                        sub,
                        len,
                        offset: chunk,
                        toggle: false,
                        data: bytes,
                    };
                    self.publish_mailbox(&reply[..10 + chunk], 3);
                }
            }
            0x20 => {
                if p.len() < 10 {
                    self.abort(index, sub, 0x0504_0001);
                    return;
                }
                reply[2] = 0x60;
                if command & 2 != 0 {
                    let len = if command & 1 != 0 {
                        4 - ((command >> 2) & 3) as usize
                    } else {
                        4
                    };
                    if self.store_download(index, sub, &p[6..6 + len]).is_err() {
                        self.abort(index, sub, 0x0604_0047);
                        return;
                    }
                } else {
                    let len = u32::from_le_bytes([p[6], p[7], p[8], p[9]]) as usize;
                    if len > MAX_OBJECT_BYTES || p.len() - 10 > len {
                        self.abort(index, sub, 0x0504_0005);
                        return;
                    }
                    let first = p.len() - 10;
                    self.transfer = Transfer {
                        active: first < len,
                        upload: false,
                        index,
                        sub,
                        len,
                        offset: first,
                        toggle: false,
                        data: [0; MAX_OBJECT_BYTES],
                    };
                    self.transfer.data[..first].copy_from_slice(&p[10..]);
                    if first == len {
                        let data = self.transfer.data;
                        if self.store_download(index, sub, &data[..len]).is_err() {
                            self.abort(index, sub, 0x0604_0047);
                            return;
                        }
                    }
                }
                self.publish_mailbox(&reply[..6], 3);
            }
            _ => self.abort(index, sub, 0x0504_0001),
        }
    }
    fn information(&mut self, size: usize) {
        // Independent application metadata for the drive's declared objects.
        let p = self.send;
        let mut reply = [0; 16];
        reply[1] = 0x80;
        if size < 10 || p[8] != 5 {
            reply[2] = 7;
            reply[6..10].copy_from_slice(&0x0602_0000u32.to_le_bytes());
            self.publish_mailbox(&reply[..10], 3);
            return;
        }
        let index = u16::from_le_bytes([p[12], p[13]]);
        let sub = p[14];
        let metadata = match (index, sub) {
            (0x7000, 1) => Some((6u16, 16u16, 0x78u16)),
            (0x6000, 1) => Some((6u16, 16u16, 0x87u16)),
            (0x603f | 0x6041, 0) => Some((6u16, 16u16, 0x87u16)),
            (0x6040, 0) => Some((6, 16, 0x78)),
            (0x6060, 0) => Some((2, 8, 0x78)),
            (0x6061, 0) => Some((2, 8, 0x87)),
            (0x6064, 0) => Some((4, 32, 0x87)),
            (0x607a | 0x60ff, 0) => Some((4, 32, 0x78)),
            (0x6071, 0) => Some((3, 16, 0x78)),
            _ => None,
        };
        if let Some((data_type, bits, access)) = metadata {
            reply[2] = 6;
            reply[6..8].copy_from_slice(&index.to_le_bytes());
            reply[8] = sub;
            reply[9] = 1;
            reply[10..12].copy_from_slice(&data_type.to_le_bytes());
            reply[12..14].copy_from_slice(&bits.to_le_bytes());
            reply[14..16].copy_from_slice(&access.to_le_bytes());
            self.publish_mailbox(&reply, 3);
        } else {
            reply[2] = 7;
            reply[6..10].copy_from_slice(&0x0602_0000u32.to_le_bytes());
            self.publish_mailbox(&reply[..10], 3);
        }
    }
    fn store_download(&mut self, index: u16, sub: u8, data: &[u8]) -> Result<(), SimError> {
        // These application objects exist and can be remapped. Other unknown
        // indices are absent rather than invented by receiving a request.
        if !matches!(index,0x1600..=0x17ff|0x1a00..=0x1bff|0x1c12|0x1c13|0x2000..=0x2007|0x6040|0x6060|0x607a|0x60ff|0x6071)
            && self.object(index, sub).is_none()
        {
            return Err(SimError::ObjectCapacity);
        }
        self.set_object(index, sub, data)
    }
    fn upload_segment(&mut self, command: u8) {
        if (command & 0x10 != 0) != self.transfer.toggle {
            self.abort(self.transfer.index, self.transfer.sub, 0x0503_0000);
            return;
        }
        let remaining = self.transfer.len - self.transfer.offset;
        let capacity = self.sm_len(1).saturating_sub(9).max(7);
        let chunk = remaining.min(capacity);
        let last = chunk == remaining;
        let wire = chunk.max(7);
        let mut reply = [0; MAILBOX];
        reply[1] = 0x30;
        reply[2] = u8::from(self.transfer.toggle) << 4
            | u8::from(last)
            | if last && chunk < 7 {
                ((7 - chunk) as u8) << 1
            } else {
                0
            };
        reply[3..3 + chunk].copy_from_slice(
            &self.transfer.data[self.transfer.offset..self.transfer.offset + chunk],
        );
        self.transfer.offset += chunk;
        self.transfer.toggle = !self.transfer.toggle;
        self.transfer.active = !last;
        self.publish_mailbox(&reply[..3 + wire], 3);
    }
    fn download_segment(&mut self, command: u8, p: &[u8]) {
        if (command & 0x10 != 0) != self.transfer.toggle {
            self.abort(self.transfer.index, self.transfer.sub, 0x0503_0000);
            return;
        }
        let last = command & 1 != 0;
        let unused = ((command >> 1) & 7) as usize;
        if p.len() < 10
            || unused > p.len() - 3
            || (!last && unused != 0)
            || (unused != 0 && p.len() != 10)
        {
            self.abort(self.transfer.index, self.transfer.sub, 0x0504_0001);
            return;
        }
        let len = p.len() - 3 - unused;
        let end = self.transfer.offset + len;
        if end > self.transfer.len || last != (end == self.transfer.len) {
            self.abort(self.transfer.index, self.transfer.sub, 0x0607_0010);
            return;
        }
        self.transfer.data[self.transfer.offset..end].copy_from_slice(&p[3..3 + len]);
        self.transfer.offset = end;
        let mut reply = [0; 3];
        reply[1] = 0x30;
        reply[2] = 0x20 | (u8::from(self.transfer.toggle) << 4);
        self.transfer.toggle = !self.transfer.toggle;
        if last {
            let t = self.transfer;
            self.transfer.active = false;
            if self
                .store_download(t.index, t.sub, &t.data[..t.len])
                .is_err()
            {
                self.abort(t.index, t.sub, 0x0604_0047);
                return;
            }
        }
        self.publish_mailbox(&reply, 3);
    }
}
