use failure::Error;

use psi::Descriptor;

#[derive(Debug)]
pub struct Event {
    pub event_id: u16,
    pub start_time: u64,
    pub duration: u32,
    pub running_status: u8,
    pub free_ca_mode: bool,
    pub descriptors: Vec<Descriptor>,
}

#[derive(Debug)]
pub struct EventInformationSection<'a> {
    pub table_id: u8,
    pub section_syntax_indicator: u8,
    pub service_id: u16,
    pub versoin_number: u8,
    pub current_next_indicator: u8,
    pub section_number: u8,
    pub last_section_number: u8,
    pub transport_stream_id: u16,
    pub original_network_id: u16,
    pub segment_last_section_number: u8,
    pub last_table_id: u8,
    pub events: Vec<Event>,

    _raw_bytes: &'a [u8],
}

impl Event {
    fn parse(bytes: &[u8]) -> Result<(Event, usize), Error> {
        check_len!(bytes.len(), 12);
        let event_id = (u16::from(bytes[0]) << 8) | u16::from(bytes[1]);
        let start_time = (u64::from(bytes[2]) << 32)
            | (u64::from(bytes[3]) << 24)
            | (u64::from(bytes[4]) << 16)
            | (u64::from(bytes[5]) << 8)
            | u64::from(bytes[6]);
        let duration =
            (u32::from(bytes[7]) << 16) | (u32::from(bytes[8]) << 8) | u32::from(bytes[9]);
        let running_status = bytes[10] >> 5;
        let free_ca_mode = (bytes[10] >> 4) & 1 > 0;
        let descriptors_loop_length = (usize::from(bytes[10] & 0xf) << 8) | usize::from(bytes[11]);
        check_len!(bytes.len() - 12, descriptors_loop_length);
        let mut bytes = &bytes[12..descriptors_loop_length + 12];
        let mut descriptors = Vec::new();
        while bytes.len() > 0 {
            let (desc, n) = Descriptor::parse(bytes)?;
            descriptors.push(desc);
            bytes = &bytes[n..];
        }
        Ok((
            Event {
                event_id,
                start_time,
                duration,
                running_status,
                free_ca_mode,
                descriptors,
            },
            descriptors_loop_length + 12,
        ))
    }
}
