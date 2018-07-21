#[macro_use]
extern crate failure;
use failure::Error;
struct AdaptationField {}
struct TSPacket<'a> {
    transport_error_indicator: bool,
    payload_unit_start_indicator: bool,
    transport_priority: bool,
    pid: u16,
    transport_scrambling_control: u8,
    continuity_counter: u8,
    adaptation_field: Option<AdaptationField>,
    data_byte: Option<&'a [u8]>,
}

const TS_PACKET_LENGTH: usize = 188;
const SYNC_BYTE: u8 = 0x47;

const PROGRAM_ASSOCIATION_TABLE: u16 = 0;
const CONDITIONAL_ACCESS_TABLE: u16 = 1;
const TRANSPORT_STREAM_DESCRIPTION_TABLE: u16 = 2;
impl<'a> TSPacket<'a> {
    fn parse(bytes: &[u8]) -> Result<TSPacket, Error> {
        if bytes.len() != TS_PACKET_LENGTH {
            bail!("bytes does not {}", TS_PACKET_LENGTH);
        }
        if bytes[0] != SYNC_BYTE {
            bail!("sync byte does not {}", SYNC_BYTE);
        }
        let transport_error_indicator = bytes[1] & 0x80 > 0;
        let payload_unit_start_indicator = bytes[1] & 0x40 > 0;
        let transport_priority = bytes[1] & 0x20 > 0;
        let pid = (bytes[1] as u16 & 0x1f << 8) | (bytes[2] as u16);
        let transport_scrambling_control = bytes[3] >> 6;
        let adaptation_field_control = bytes[3] & 0x30 >> 4;
        let continuity_counter = bytes[3] & 0xf;
        let (adaptation_field, adaptation_field_length) = match adaptation_field_control {
            0b10 | 0b11 => {
                let (af, n) = AdaptationField::parse(&bytes[4..])?;
                (Some(af), n)
            }
            _ => (None, 0),
        };
        let data_byte = match adaptation_field_control {
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
            data_byte,
        })
    }
}

impl AdaptationField {
    fn parse(bytes: &[u8]) -> Result<(AdaptationField, usize), Error> {
        let adaptation_field_length = bytes[0];
        if bytes.len() < adaptation_field_length + 1 {
            bail!(
                "adaptation_field_length({}) is bigger than bytes()",
                adaptation_field_control + 1,
                bytes.len()
            );
        }
        Ok((AdaptationField, adaptation_field_length + 1))
    }
}

fn main() {
    println!("Hello, world!");
}
