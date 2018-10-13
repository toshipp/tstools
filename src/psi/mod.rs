use failure::Error;
use std::collections::HashMap;

use crc32;
use ts;
use util;

pub const PROGRAM_ASSOCIATION_SECTION: u8 = 0;
pub const CONDITIONAL_ACCESS_SECTION: u8 = 1;
pub const TS_PROGRAM_MAP_SECTION: u8 = 2;

#[derive(Debug)]
enum BufferState {
    Initial,
    Skipping(u8),
    Buffering(Option<u16>),
    Proceeded,
}

#[derive(Debug)]
pub struct Buffer {
    state: BufferState,
    counter: u8,
    buf: Vec<u8>,
}

impl Buffer {
    pub fn new() -> Buffer {
        Buffer {
            state: BufferState::Initial,
            counter: 0,
            buf: Vec::new(),
        }
    }
    pub fn feed(&mut self, packet: &ts::TSPacket) -> Result<Option<&[u8]>, Error> {
        if packet.transport_error_indicator {
            return Ok(None);
        }
        if packet.data_bytes.is_none() {
            bail!("malformed psi packet, no data")
        }
        let mut bytes = packet.data_bytes.unwrap();

        if packet.payload_unit_start_indicator {
            let pointer_field = bytes[0];
            self.state = BufferState::Skipping(pointer_field + 1);
            self.counter = packet.continuity_counter;
            self.buf.clear();
        } else {
            match self.state {
                BufferState::Initial => {
                    // seen partial section
                    return Ok(None);
                }
                BufferState::Proceeded => {
                    // already completed
                    return Ok(None);
                }
                _ => {
                    if self.counter == packet.continuity_counter {
                        // duplicate packet
                        return Ok(None);
                    } else if (self.counter + 1) % 16 == packet.continuity_counter {
                        self.counter = packet.continuity_counter;
                    } else {
                        self.state = BufferState::Initial;
                        bail!("psi packet discontinued");
                    }
                }
            }
        }

        if let BufferState::Skipping(n) = self.state {
            if bytes.len() < (n as usize) {
                self.state = BufferState::Skipping(n - (bytes.len() as u8));
                return Ok(None);
            } else {
                self.state = BufferState::Buffering(None);
                bytes = &bytes[n as usize..];
            }
        }
        if let BufferState::Buffering(mut length) = self.state {
            self.buf.extend_from_slice(bytes);
            if length.is_none() && self.buf.len() >= 3 {
                let section_length = (u16::from(bytes[1] & 0xf) << 8) | u16::from(bytes[2]) + 3;
                self.state = BufferState::Buffering(Some(section_length));
                length = Some(section_length);
            }
            if let Some(length) = length {
                if self.buf.len() >= (length as usize) {
                    self.state = BufferState::Proceeded;
                    return Ok(Some(&self.buf[..length as usize]));
                }
            }
        }
        return Ok(None);
    }
}

#[derive(Debug)]
pub enum Descriptor {
    Descriptor(u8),
}

impl Descriptor {
    fn parse(bytes: &[u8]) -> Result<(Descriptor, usize), Error> {
        check_len!(bytes.len(), 2);
        let descriptor_tag = bytes[0];
        let descriptor_length = usize::from(bytes[1]);
        return Ok((
            Descriptor::Descriptor(descriptor_tag),
            descriptor_length + 2,
        ));
    }
}

#[derive(Debug)]
pub struct StreamInfo {
    pub stream_type: u8,
    pub elementary_pid: u16,
    pub descriptors: Vec<Descriptor>,
}

impl StreamInfo {
    fn parse(bytes: &[u8]) -> Result<(StreamInfo, usize), Error> {
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

    fn calculate_crc32(&self) -> u32 {
        return crc32::crc32(self._raw_bytes);
    }
}

#[derive(Debug)]
pub struct TSProgramMapSection {
    pub table_id: u8,
    pub section_syntax_indicator: u8,
    pub program_number: u16,
    pub version_number: u8,
    pub current_next_indicator: u8,
    pub section_number: u8,
    pub last_section_number: u8,
    pub pcr_pid: u16,
    pub descriptors: Vec<Descriptor>,
    pub stream_info: Vec<StreamInfo>,
    pub crc_32: u32,
}

impl TSProgramMapSection {
    pub fn parse(bytes: &[u8]) -> Result<TSProgramMapSection, Error> {
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
