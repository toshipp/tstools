use failure::Error;

pub const TS_PACKET_LENGTH: usize = 188;
const SYNC_BYTE: u8 = 0x47;

#[derive(Debug)]
pub struct AdaptationField {
    adaptation_field_length: u8,
}
#[derive(Debug)]
pub struct TSPacket<'a> {
    pub transport_error_indicator: bool,
    pub payload_unit_start_indicator: bool,
    pub transport_priority: bool,
    pub pid: u16,
    pub transport_scrambling_control: u8,
    pub continuity_counter: u8,
    pub adaptation_field: Option<AdaptationField>,
    pub data_bytes: Option<&'a [u8]>,
    raw_bytes: &'a [u8],
}
impl<'a> TSPacket<'a> {
    pub fn parse(bytes: &[u8]) -> Result<TSPacket, Error> {
        if bytes.len() != TS_PACKET_LENGTH {
            bail!("bytes does not {}", TS_PACKET_LENGTH);
        }
        if bytes[0] != SYNC_BYTE {
            bail!("sync byte does not {}", SYNC_BYTE);
        }
        let transport_error_indicator = bytes[1] & 0x80 > 0;
        let payload_unit_start_indicator = bytes[1] & 0x40 > 0;
        let transport_priority = bytes[1] & 0x20 > 0;
        let pid = (u16::from(bytes[1] & 0x1f) << 8) | u16::from(bytes[2]);
        let transport_scrambling_control = bytes[3] >> 6;
        let adaptation_field_control = (bytes[3] & 0x30) >> 4;
        let continuity_counter = bytes[3] & 0xf;
        let (adaptation_field, adaptation_field_length) = match adaptation_field_control {
            0b10 | 0b11 => {
                let (af, n) = AdaptationField::parse(&bytes[4..])?;
                (Some(af), n)
            }
            _ => (None, 0),
        };
        let data_bytes = match adaptation_field_control {
            0b01 | 0b11 => Some(&bytes[4 + adaptation_field_length..]),
            _ => None,
        };
        Ok(TSPacket {
            transport_error_indicator,
            payload_unit_start_indicator,
            transport_priority,
            pid,
            transport_scrambling_control,
            continuity_counter,
            adaptation_field,
            data_bytes,
            raw_bytes: bytes,
        })
    }
}

impl AdaptationField {
    fn parse(bytes: &[u8]) -> Result<(AdaptationField, usize), Error> {
        if bytes.len() < 1 {
            bail!("too short for adaptation field");
        }
        let adaptation_field_length = usize::from(bytes[0]);
        if bytes.len() < adaptation_field_length + 1 {
            bail!(
                "adaptation_field_length({}) is bigger than bytes({})",
                adaptation_field_length + 1,
                bytes.len()
            );
        }
        Ok((
            AdaptationField {
                adaptation_field_length: adaptation_field_length as u8,
            },
            adaptation_field_length + 1,
        ))
    }
}
