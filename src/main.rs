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
        if bytes.len() < 1 {
            bail!("too short for adaptation field");
        }
        let adaptation_field_length = bytes[0] as usize;
        if bytes.len() < adaptation_field_length + 1 {
            bail!(
                "adaptation_field_length({}) is bigger than bytes({})",
                adaptation_field_length + 1,
                bytes.len()
            );
        }
        Ok((AdaptationField {}, adaptation_field_length + 1))
    }
}

const PROGRAM_STREAM_MAP: u8 = 0b10111100;
const PRIVATE_STREAM_2: u8 = 0b10111111;
const ECM: u8 = 0b11110000;
const EMM: u8 = 0b11110001;
const PROGRAM_STREAM_DIRECTORY: u8 = 0b11111111;
const DSMCC_STREAM: u8 = 0b11110010;
const ITU_T_REC_H_222_1_TYPE_E_STREAM: u8 = 0b11111000;
const PADDING_STREAM: u8 = 0b10111110;

struct PESPacket<'a> {
    packet_start_code_prefix: u32,
    stream_id: u8,
    body: PESPacketBody<'a>,
}

enum DSMTrickMode {}
struct Todo {}
struct PESPacketExtension {
    pes_private_data: Todo,
    pack_header: Todo,
    program_packet_sequence_counter: Option<u8>,
    mpeg1_mpeg2_identifier: Option<u8>,
    original_stuff_length: Todo,
    p_std_buffer_scale: Todo,
    p_std_buffer_size: Todo,
}

struct ESCR {
    base: u32,
    extension: u16,
}

struct NormalPESPacketBody<'a> {
    pes_scrambling_control: u8,
    pes_priority: u8,
    data_alignment_indicator: u8,
    copyright: u8,
    original_or_copy: u8,
    pts_dts_flags: u8,
    escr_flag: u8,
    es_rate_flag: u8,
    dsm_trick_mode_flag: u8,
    additional_copy_info_flag: u8,
    pes_crc_flag: u8,
    pes_extension_flag: u8,
    pes_header_data_length: u8,
    pts: Option<u32>,
    dts: Option<u32>,
    escr: Option<ESCR>,
    es_rate: Option<u32>,
    dsm_trick_mode: DSMTrickMode,
    additional_copy_info: u8,
    previous_pes_packet_crc: u16,
    pes_extension: Option<PESPacketExtension>,
    pes_packet_data_byte: &'a [u8],
}
enum PESPacketBody<'a> {
    NormalPESPacketBody(NormalPESPacketBody<'a>),
    DataBytes(&'a [u8]),
    PaddingByte,
}

impl<'a> PESPacket<'a> {
    fn parse(bytes: &[u8]) -> Result<PESPacket, Error> {
        if bytes.len() < 3 + 1 + 2 {
            bail!("too short for PES packet {}", bytes.len());
        }
        let packet_start_code_prefix =
            (bytes[0] as u32) << 16 | (bytes[1] as u32) << 8 | (bytes[2] as u32);
        let stream_id = bytes[3];
        let pes_packet_length = (bytes[4] as usize) << 8 | (bytes[5] as usize);
        let body = match stream_id {
            PROGRAM_STREAM_MAP
            | PRIVATE_STREAM_2
            | ECM
            | EMM
            | PROGRAM_STREAM_DIRECTORY
            | DSMCC_STREAM
            | ITU_T_REC_H_222_1_TYPE_E_STREAM => {
                PESPacketBody::DataBytes(&bytes[6..6 + pes_packet_length])
            }
            PADDING_STREAM => PESPacketBody::PaddingByte,
            _ => PESPacketBody::NormalPESPacketBody(NormalPESPacketBody::parse(
                &bytes[6..6 + pes_packet_length],
            )?),
        };
        Ok(PESPacket {
            packet_start_code_prefix,
            stream_id,
            body,
        })
    }
}

impl<'a> NormalPESPacketBody<'a> {
    fn parse(bytes: &[u8]) -> Result<NormalPESPacketBody, Error> {
        if bytes.len() < 3 {
            bail!("too short for pes packet {}", bytes.len());
        }
        let pes_scrambling_control = bytes[0] >> 6 & 3;
        let pes_priority = bytes[0] >> 5 & 1;
        let data_alignment_indicator = bytes[0] >> 4 & 1;
        let copyright = bytes[0] >> 3 & 1;
        let original_or_copy = bytes[0] >> 2 & 1;
        let pts_dts_flags = bytes[1] >> 6 & 3;
        let escr_flag = bytes[1] >> 5 & 1;
        let es_rate_flag = bytes[1] >> 4 & 1;
        let dsm_trick_mode_flag = bytes[1] >> 3 & 1;
        let additional_copy_info_flag = bytes[1] >> 2 & 1;
        let pes_crc_flag = bytes[1] >> 1 & 1;
        let pes_extension_flag = bytes[1] & 1;
        let pes_header_data_length = bytes[2];
        let mut bytes = &bytes[3..];
        let (pts, dts) = match pts_dts_flags {
            0b10 => {
                let pts = NormalPESPacketBody::parse_timestamp(bytes)?;
                bytes = &bytes[5..];
                (Some(pts), None)
            }
            0b11 => {
                let pts = NormalPESPacketBody::parse_timestamp(&bytes[0..])?;
                let dts = NormalPESPacketBody::parse_timestamp(&bytes[5..])?;
                bytes = &bytes[10..];
                (Some(pts), Some(dts))
            }
            _ => (None, None),
        };
        let escr = match escr_flag {
            1 => {
                if bytes.len() < 6 {
                    bail!("too short for ESCR");
                }
                Some(ESCR {
                    base: 0,
                    extension: 0,
                })
            }
            _ => None,
        };

        unimplemented!();
    }

    fn parse_timestamp(bytes: &[u8]) -> Result<u32, Error> {
        if bytes.len() < 5 {
            bail!("too short for timestamp {}", bytes.len());
        }
        Ok((bytes[0] as u32) >> 1 & 0b111
            | (bytes[1] as u32)
            | (bytes[2] as u32) >> 1
            | (bytes[3] as u32)
            | (bytes[4] as u32) >> 1)
    }
}

fn main() {
    println!("Hello, world!");
}
