use anyhow::{bail, Result};

use crate::util;

use crate::psi::descriptor::Descriptor;

pub const STREAM_TYPE_VIDEO: u8 = 0x2;
pub const STREAM_TYPE_PES_PRIVATE_DATA: u8 = 0x6;
pub const STREAM_TYPE_ADTS: u8 = 0xf;
pub const STREAM_TYPE_H264: u8 = 0x1b;

#[derive(Debug)]
pub struct StreamInfo<'a> {
    pub stream_type: u8,
    pub elementary_pid: u16,
    pub descriptors: Vec<Descriptor<'a>>,
}

impl<'a> StreamInfo<'a> {
    fn parse(bytes: &[u8]) -> Result<(StreamInfo<'_>, usize)> {
        check_len!(bytes.len(), 5);
        let stream_type = bytes[0];
        let elementary_pid = (u16::from(bytes[1] & 0x1f) << 8) | u16::from(bytes[2]);
        let es_info_length = (usize::from(bytes[3] & 0xf) << 8) | usize::from(bytes[4]);
        check_len!(bytes.len(), 5 + es_info_length);
        let mut descriptors = vec![];
        let mut bytes = &bytes[5..5 + es_info_length];
        while bytes.len() > 0 {
            let (descriptor, n) = Descriptor::parse(bytes)?;
            descriptors.push(descriptor);
            check_len!(bytes.len(), n);
            bytes = &bytes[n..];
        }
        Ok((
            StreamInfo {
                stream_type,
                elementary_pid,
                descriptors,
            },
            5 + es_info_length,
        ))
    }
}

#[derive(Debug)]
pub struct TSProgramMapSection<'a> {
    pub table_id: u8,
    pub section_syntax_indicator: u8,
    pub program_number: u16,
    pub version_number: u8,
    pub current_next_indicator: u8,
    pub section_number: u8,
    pub last_section_number: u8,
    pub pcr_pid: u16,
    pub descriptors: Vec<Descriptor<'a>>,
    pub stream_info: Vec<StreamInfo<'a>>,
    pub crc_32: u32,
}

impl<'a> TSProgramMapSection<'a> {
    pub fn parse(bytes: &[u8]) -> Result<TSProgramMapSection<'_>> {
        let table_id = bytes[0];
        if table_id != 0x02 {
            bail!("table_id should 0x02, {}", table_id);
        }
        let section_syntax_indicator = bytes[1] >> 7;
        let section_length = (usize::from(bytes[1] & 0xf) << 8) | usize::from(bytes[2]);
        assert!(section_length < 1021);
        let program_number = (u16::from(bytes[3]) << 8) | u16::from(bytes[4]);
        let version_number = (bytes[5] & 0x3e) >> 1;
        let current_next_indicator = bytes[5] & 0x1;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let pcr_pid = (u16::from(bytes[8] & 0x1f) << 8) | u16::from(bytes[9]);
        let program_info_length = (usize::from(bytes[10] & 0xf) << 8) | usize::from(bytes[11]);

        check_len!(bytes.len(), 3 + section_length);
        check_len!(bytes.len(), 12 + program_info_length);
        let mut descriptors = vec![];
        {
            let mut bytes = &bytes[12..12 + program_info_length];
            while bytes.len() > 0 {
                let (descriptor, n) = Descriptor::parse(bytes)?;
                descriptors.push(descriptor);
                bytes = &bytes[n..];
            }
        }

        let mut stream_info = vec![];
        {
            let mut bytes = &bytes[12 + program_info_length..3 + section_length - 4];
            while bytes.len() > 0 {
                let (info, n) = StreamInfo::parse(bytes)?;
                stream_info.push(info);
                check_len!(bytes.len(), n);
                bytes = &bytes[n..];
            }
        }
        let crc_32 = util::read_u32(&bytes[3 + section_length - 4..])?;
        return Ok(TSProgramMapSection {
            table_id,
            section_syntax_indicator,
            program_number,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            pcr_pid,
            descriptors,
            stream_info,
            crc_32,
        });
    }
}
