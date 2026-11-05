//! Rebuild physical mappings from verified SII defaults and frozen logical PDOs.

use crate::{
    MappingTable, PdoDirection, ProductMappingError, ProductSlaveConfig, StartupController,
    StaticProductConfig,
};
use esop_ethercat_core::{FmmuConfig, MAX_ESC_SYNC_MANAGERS};

impl<const SLAVES: usize, const DOMAINS: usize, const AXES: usize>
    StaticProductConfig<'_, SLAVES, DOMAINS, AXES>
{
    pub fn build_mapping_for_slave<const SMS: usize, const FMMUS: usize>(
        &self,
        position: u16,
        startup: &StartupController<SLAVES>,
    ) -> Result<MappingTable<SMS, FMMUS>, ProductMappingError> {
        let slave = self
            .slaves
            .iter()
            .find(|slave| slave.position == position)
            .ok_or(ProductMappingError::UnknownSlave(position))?;
        let record = startup
            .records()
            .iter()
            .find(|record| record.position == position)
            .ok_or(ProductMappingError::UnverifiedSii(position))?;
        if record.station_address != slave.station_address
            || !record.identity.matches(slave.identity)
        {
            return Err(ProductMappingError::IdentityMismatch(position));
        }
        let expected = self
            .sii_configuration_signature(*slave)
            .map_err(ProductMappingError::Startup)?;
        if startup.verified_sii(position) != Some(expected) {
            return Err(ProductMappingError::UnverifiedSii(position));
        }
        let physical = startup
            .verified_sii_mapping(position)
            .ok_or(ProductMappingError::UnverifiedSii(position))?;
        self.build_mapping_from_sii(*slave, physical)
    }

    pub(crate) fn build_mapping_from_sii<const SMS: usize, const FMMUS: usize>(
        &self,
        slave: ProductSlaveConfig,
        physical: &MappingTable<MAX_ESC_SYNC_MANAGERS, 0>,
    ) -> Result<MappingTable<SMS, FMMUS>, ProductMappingError> {
        let domain = self
            .domains
            .iter()
            .find(|domain| domain.config.id == slave.domain_id)
            .ok_or(ProductMappingError::UnknownDomain(slave.domain_id))?;
        let invalid = ProductMappingError::InvalidPdoLayout(slave.position);
        let mut table = MappingTable::new();
        let mut sm_bits = [0usize; MAX_ESC_SYNC_MANAGERS];
        let mut sm_end = [None; MAX_ESC_SYNC_MANAGERS];
        let mut sm_direction = [None; MAX_ESC_SYNC_MANAGERS];
        for pdo in self
            .pdos
            .iter()
            .filter(|pdo| pdo.request.slave_position == slave.position)
        {
            let sm = usize::from(pdo.sync_manager);
            if sm >= MAX_ESC_SYNC_MANAGERS || pdo.domain_id != slave.domain_id {
                return Err(invalid);
            }
            if sm_end[sm].is_some_and(|end| end != pdo.bit_offset)
                || sm_direction[sm].is_some_and(|direction| direction != pdo.request.direction)
            {
                return Err(invalid);
            }
            sm_end[sm] = Some(
                pdo.bit_offset
                    .checked_add(usize::from(pdo.request.bit_length))
                    .ok_or(invalid)?,
            );
            sm_direction[sm] = Some(pdo.request.direction);
            sm_bits[sm] = sm_bits[sm]
                .checked_add(usize::from(pdo.request.bit_length))
                .ok_or(invalid)?;
        }
        for mut sm in physical.sync_managers().iter().copied() {
            let index = usize::from(sm.index);
            if index >= MAX_ESC_SYNC_MANAGERS {
                return Err(invalid);
            }
            let mailbox = slave.mailbox().is_some()
                && (sm.index == slave.mailbox_send_sync_manager
                    || sm.index == slave.mailbox_receive_sync_manager.index);
            if mailbox {
                let (address, capacity, control) = if sm.index == slave.mailbox_send_sync_manager {
                    (
                        slave.mailbox_config.send_address,
                        slave.mailbox_config.send_capacity,
                        slave.mailbox_send_control_byte,
                    )
                } else {
                    let receive = slave.mailbox_receive_sync_manager;
                    (receive.physical_start, receive.length, receive.control)
                };
                if (sm.physical_start, sm.length, sm.control) != (address, capacity, control)
                    || sm_bits[index] != 0
                    || !sm.enable
                {
                    return Err(ProductMappingError::MailboxMismatch(sm.index));
                }
            } else if sm_bits[index] != 0 {
                let length = u16::try_from(sm_bits[index].div_ceil(8)).map_err(|_| invalid)?;
                if length > sm.length || !sm.enable {
                    return Err(ProductMappingError::SyncManagerTooSmall(sm.index));
                }
                sm.length = length;
            } else {
                sm.enable = false;
            }
            table
                .add_sync_manager(sm)
                .map_err(ProductMappingError::Mapping)?;
            if let Some(activation) = slave.op_only_outputs.activation_template(sm.index) {
                table
                    .mark_op_only_output(sm.index, activation)
                    .map_err(ProductMappingError::Mapping)?;
            }
        }

        let mut physical_offsets = [0usize; MAX_ESC_SYNC_MANAGERS];
        let mut fmmu_index = 0u8;
        for direction in [PdoDirection::Rx, PdoDirection::Tx] {
            let mut entries = self
                .pdos
                .iter()
                .filter(|pdo| {
                    pdo.request.slave_position == slave.position
                        && pdo.request.direction == direction
                })
                .peekable();
            while let Some(first) = entries.next() {
                let sm = table
                    .sync_manager(first.sync_manager)
                    .map_err(ProductMappingError::Mapping)?;
                let mut bits = usize::from(first.request.bit_length);
                while let Some(next) = entries.peek() {
                    if next.assignment_index != first.assignment_index {
                        break;
                    }
                    if next.sync_manager != first.sync_manager
                        || next.bit_offset != first.bit_offset + bits
                    {
                        return Err(invalid);
                    }
                    bits = bits
                        .checked_add(usize::from(next.request.bit_length))
                        .ok_or(invalid)?;
                    entries.next();
                }
                let offset = physical_offsets[usize::from(first.sync_manager)];
                let end = first.bit_offset.checked_add(bits).ok_or(invalid)?;
                if end > domain.config.process_image_len * 8 || bits == 0 {
                    return Err(invalid);
                }
                let physical_start = sm
                    .physical_start
                    .checked_add(u16::try_from(offset / 8).map_err(|_| invalid)?)
                    .ok_or(invalid)?;
                let logical_start = domain
                    .config
                    .logical_address
                    .checked_add(u32::try_from(first.bit_offset / 8).map_err(|_| invalid)?)
                    .ok_or(invalid)?;
                table
                    .add_fmmu(FmmuConfig {
                        index: fmmu_index,
                        logical_start,
                        length: u16::try_from(end.div_ceil(8) - first.bit_offset / 8)
                            .map_err(|_| invalid)?,
                        logical_start_bit: (first.bit_offset % 8) as u8,
                        logical_end_bit: ((end - 1) % 8) as u8,
                        physical_start,
                        physical_start_bit: (offset % 8) as u8,
                        fmmu_type: if direction == PdoDirection::Rx { 2 } else { 1 },
                        enable: true,
                    })
                    .map_err(ProductMappingError::Mapping)?;
                physical_offsets[usize::from(first.sync_manager)] =
                    offset.checked_add(bits).ok_or(invalid)?;
                fmmu_index = fmmu_index.checked_add(1).ok_or(invalid)?;
            }
        }
        if let Some(status) = slave.mapped_mailbox_status {
            table
                .add_fmmu(status.fmmu)
                .map_err(ProductMappingError::Mapping)?;
        }
        Ok(table)
    }
}
