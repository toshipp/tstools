use anyhow::{bail, Error, Result};
use bytes::{Bytes, BytesMut};
use tokio_util::codec::Decoder;

pub const TS_PACKET_LENGTH: usize = 188;
const SYNC_BYTE: u8 = 0x47;

#[derive(Debug, Clone)]
pub struct AdaptationField {
    pub raw: Bytes,
}

#[derive(Debug, Clone)]
pub struct TSPacket {
    pub transport_error_indicator: bool,
    pub payload_unit_start_indicator: bool,
    pub transport_priority: bool,
    pub pid: u16,
    pub transport_scrambling_control: u8,
    pub adaptation_field_control: u8,
    pub continuity_counter: u8,
    pub adaptation_field: Option<AdaptationField>,
    pub data: Option<Bytes>,
    raw: Bytes,
}

impl TSPacket {
    pub fn into_raw(self) -> Bytes {
        self.raw
    }
}

pub struct TSPacketDecoder {}

impl TSPacketDecoder {
    pub fn new() -> Self {
        TSPacketDecoder {}
    }
}

impl Decoder for TSPacketDecoder {
    type Item = TSPacket;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>> {
        if src.len() < TS_PACKET_LENGTH {
            return Ok(None);
        }
        if src[0] != SYNC_BYTE {
            bail!("sync byte does not {}", SYNC_BYTE);
        }
        let src = src.split_to(TS_PACKET_LENGTH).freeze();
        let transport_error_indicator = src[1] & 0x80 > 0;
        let payload_unit_start_indicator = src[1] & 0x40 > 0;
        let transport_priority = src[1] & 0x20 > 0;
        let pid = (u16::from(src[1] & 0x1f) << 8) | u16::from(src[2]);
        let transport_scrambling_control = src[3] >> 6;
        let adaptation_field_control = (src[3] & 0x30) >> 4;
        let continuity_counter = src[3] & 0xf;
        // FIXME: return error.
        if transport_error_indicator {
            return Ok(Some(TSPacket {
                transport_error_indicator,
                payload_unit_start_indicator,
                transport_priority,
                pid,
                transport_scrambling_control,
                adaptation_field_control,
                continuity_counter,
                adaptation_field: None,
                data: None,
                raw: src,
            }));
        }
        let (adaptation_field, adaptation_field_length) = match adaptation_field_control {
            0b10 | 0b11 => {
                let (af, n) = AdaptationField::decode(&mut src.clone().split_off(4))?;
                (Some(af), n)
            }
            _ => (None, 0),
        };
        let data = match adaptation_field_control {
            0b01 | 0b11 => Some(src.clone().split_off(4 + adaptation_field_length)),
            _ => None,
        };
        Ok(Some(TSPacket {
            transport_error_indicator,
            payload_unit_start_indicator,
            transport_priority,
            pid,
            transport_scrambling_control,
            adaptation_field_control,
            continuity_counter,
            adaptation_field,
            data,
            raw: src,
        }))
    }
}

impl AdaptationField {
    fn decode(src: &mut Bytes) -> Result<(AdaptationField, usize)> {
        check_len!(src.len(), 1);
        let adaptation_field_length = usize::from(src[0]);
        check_len!(src.len(), adaptation_field_length + 1);
        Ok((
            AdaptationField {
                raw: src.split_to(adaptation_field_length + 1),
            },
            adaptation_field_length + 1,
        ))
    }
}
