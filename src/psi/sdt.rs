use anyhow::{bail, Result};

use crate::psi::Descriptor;
use crate::util;

pub const SDT_PID: u16 = 0x0011;
pub const SELF_STREAM_TABLE_ID: u8 = 0x42;
#[allow(dead_code)]
pub const OTHER_STREAM_TABLE_ID: u8 = 0x46;

#[derive(Debug)]
pub struct Service<'a> {
    pub service_id: u16,
    pub eit_user_defined_flags: u8,
    pub eit_schedule_flag: u8,
    pub eit_present_following_flag: u8,
    pub running_status: u8,
    pub free_ca_mode: u8,
    pub descriptors: Vec<Descriptor<'a>>,
}

impl Service<'_> {
    fn parse(bytes: &[u8]) -> Result<(Service<'_>, usize)> {
        check_len!(bytes.len(), 5);
        let service_id = (u16::from(bytes[0]) << 8) | u16::from(bytes[1]);
        let eit_user_defined_flags = (bytes[2] >> 2) & 0x7;
        let eit_schedule_flag = (bytes[2] >> 1) & 0x1;
        let eit_present_following_flag = bytes[2] & 0x1;
        let running_status = bytes[3] >> 5;
        let free_ca_mode = (bytes[3] >> 4) & 0x1;
        let descriptors_loop_length = (usize::from(bytes[3] & 0xf) << 8) | usize::from(bytes[4]);
        let mut descriptors = Vec::new();
        {
            let mut bytes = &bytes[5..5 + descriptors_loop_length];
            while bytes.len() > 0 {
                let (descriptor, n) = Descriptor::parse(bytes)?;
                descriptors.push(descriptor);
                bytes = &bytes[n..];
            }
        }
        Ok((
            Service {
                service_id,
                eit_user_defined_flags,
                eit_schedule_flag,
                eit_present_following_flag,
                running_status,
                free_ca_mode,
                descriptors,
            },
            5 + descriptors_loop_length,
        ))
    }
}

#[derive(Debug)]
pub struct ServiceDescriptionSection<'a> {
    pub table_id: u8,
    pub section_syntax_indicator: u8,
    pub transport_stream_id: u16,
    pub version_number: u8,
    pub current_next_indicator: u8,
    pub section_number: u8,
    pub last_section_number: u8,
    pub original_network_id: u16,
    pub services: Vec<Service<'a>>,
    pub crc32: u32,

    _raw_bytes: &'a [u8],
}

impl ServiceDescriptionSection<'_> {
    pub fn parse(bytes: &[u8]) -> Result<ServiceDescriptionSection<'_>> {
        check_len!(bytes.len(), 11);
        let table_id = bytes[0];
        let section_syntax_indicator = bytes[1] >> 7;
        let section_length = (usize::from(bytes[1] & 0xf) << 8) | usize::from(bytes[2]);
        let transport_stream_id = (u16::from(bytes[3]) << 8) | u16::from(bytes[4]);
        let version_number = (bytes[5] >> 1) & 0x1f;
        let current_next_indicator = bytes[5] & 0x1;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let original_network_id = (u16::from(bytes[8]) << 8) | u16::from(bytes[9]);
        let mut services = Vec::new();
        {
            let mut bytes = &bytes[11..3 + section_length - 4];
            while bytes.len() > 0 {
                let (service, n) = Service::parse(bytes)?;
                services.push(service);
                bytes = &bytes[n..];
            }
        }
        let crc32 = util::read_u32(&bytes[3 + section_length - 4..])?;
        Ok(ServiceDescriptionSection {
            table_id,
            section_syntax_indicator,
            transport_stream_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            original_network_id,
            services,
            crc32,
            _raw_bytes: bytes,
        })
    }
}
