use failure::Error;
use std::collections::HashMap;

use crate::crc32;
use crate::util;

#[derive(Debug)]
pub struct ProgramAssociationSection<'a> {
    pub table_id: u8,
    pub section_syntax_indicator: u8,
    pub transport_stream_id: u16,
    pub version_number: u8,
    pub current_next_indicator: u8,
    pub section_number: u8,
    pub last_section_number: u8,
    pub program_association: HashMap<u16, u16>,
    pub crc_32: u32,

    _raw_bytes: &'a [u8],
}

impl<'a> ProgramAssociationSection<'a> {
    pub fn parse(bytes: &[u8]) -> Result<ProgramAssociationSection, Error> {
        let table_id = bytes[0];
        if table_id != 0 {
            bail!("invalid table_id: {}", table_id);
        }
        let section_syntax_indicator = bytes[1] >> 7;
        let section_length = (usize::from(bytes[1] & 0xf) << 8) | usize::from(bytes[2]);
        assert!(section_length <= 1021);
        let transport_stream_id = (u16::from(bytes[3]) << 8) | u16::from(bytes[4]);
        let version_number = (bytes[5] & 0x3e) >> 1;
        let current_next_indicator = bytes[5] & 1;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        check_len!(bytes.len(), 3 + section_length);
        let mut map = &bytes[8..3 + section_length - 4];
        let mut program_association = HashMap::new();
        if map.len() % 4 != 0 {
            bail!("invalid length");
        }
        while map.len() > 0 {
            let program_number = (u16::from(map[0]) << 8) | u16::from(map[1]);
            let pid = (u16::from(map[2] & 0x1f) << 8) | u16::from(map[3]);
            program_association.insert(program_number, pid);
            map = &map[4..];
        }

        let crc_bytes = &bytes[3 + section_length - 4..];
        let crc_32 = util::read_u32(crc_bytes)?;

        Ok(ProgramAssociationSection {
            table_id,
            section_syntax_indicator,
            transport_stream_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            program_association,
            crc_32,
            _raw_bytes: &bytes[..3 + section_length],
        })
    }

    #[allow(dead_code)]
    fn calculate_crc32(&self) -> u32 {
        return crc32::crc32(self._raw_bytes);
    }
}
