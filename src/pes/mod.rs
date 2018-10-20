use failure::Error;

mod buffer;
pub use self::buffer::*;

const PROGRAM_STREAM_MAP: u8 = 0b10111100;
const PRIVATE_STREAM_2: u8 = 0b10111111;
const ECM: u8 = 0b11110000;
const EMM: u8 = 0b11110001;
const PROGRAM_STREAM_DIRECTORY: u8 = 0b11111111;
const DSMCC_STREAM: u8 = 0b11110010;
const ITU_T_REC_H_222_1_TYPE_E_STREAM: u8 = 0b11111000;
const PADDING_STREAM: u8 = 0b10111110;

#[derive(Debug)]
struct Todo {}

type DSMTrickMode = Todo;
#[derive(Debug)]
struct ESCR {
    base: u64,
    extension: u16,
}

#[derive(Debug)]
pub struct PESPacket<'a> {
    pub packet_start_code_prefix: u32,
    pub stream_id: u8,
    pub body: PESPacketBody<'a>,
}

#[derive(Debug)]
struct PESPacketExtension<'a> {
    pes_private_data: Option<&'a [u8]>,
    pack_header: Option<&'a [u8]>,
    program_packet_sequence_counter: Option<u8>,
    mpeg1_mpeg2_identifier: Option<u8>,
    original_stuff_length: Option<u8>,
    p_std_buffer_scale: Option<u8>,
    p_std_buffer_size: Option<u16>,
}

#[derive(Debug)]
pub struct NormalPESPacketBody<'a> {
    pes_scrambling_control: u8,
    pes_priority: u8,
    data_alignment_indicator: u8,
    copyright: u8,
    original_or_copy: u8,
    pts: Option<u64>,
    dts: Option<u64>,
    escr: Option<ESCR>,
    es_rate: Option<u32>,
    dsm_trick_mode: Option<DSMTrickMode>,
    additional_copy_info: Option<u8>,
    previous_pes_packet_crc: Option<u16>,
    pes_extension: Option<PESPacketExtension<'a>>,
    pes_packet_data_byte: &'a [u8],
}

#[derive(Debug)]
pub enum PESPacketBody<'a> {
    NormalPESPacketBody(NormalPESPacketBody<'a>),
    DataBytes(&'a [u8]),
    PaddingByte,
}

impl<'a> PESPacket<'a> {
    pub fn parse(bytes: &[u8]) -> Result<PESPacket, Error> {
        if bytes.len() < 3 + 1 + 2 {
            bail!("too short for PES packet {}", bytes.len());
        }
        let packet_start_code_prefix =
            (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        let stream_id = bytes[3];
        let mut pes_packet_length = (usize::from(bytes[4]) << 8) | usize::from(bytes[5]);
        // TODO
        if pes_packet_length == 0 {
            pes_packet_length = bytes.len() - 6;
        }
        check_len!(bytes.len(), 6 + pes_packet_length);
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
        let pes_scrambling_control = (bytes[0] >> 6) & 3;
        let pes_priority = (bytes[0] >> 5) & 1;
        let data_alignment_indicator = (bytes[0] >> 4) & 1;
        let copyright = (bytes[0] >> 3) & 1;
        let original_or_copy = (bytes[0] >> 2) & 1;
        let pts_dts_flags = (bytes[1] >> 6) & 3;
        let escr_flag = (bytes[1] >> 5) & 1;
        let es_rate_flag = (bytes[1] >> 4) & 1;
        let dsm_trick_mode_flag = (bytes[1] >> 3) & 1;
        let additional_copy_info_flag = (bytes[1] >> 2) & 1;
        let pes_crc_flag = (bytes[1] >> 1) & 1;
        let pes_extension_flag = bytes[1] & 1;
        let pes_header_data_length = usize::from(bytes[2]);
        let (
            pts,
            dts,
            escr,
            es_rate,
            dsm_trick_mode,
            additional_copy_info,
            previous_pes_packet_crc,
            pes_extension,
        ) = NormalPESPacketBody::parse_optional_fields(
            &bytes[3..],
            pts_dts_flags,
            escr_flag,
            es_rate_flag,
            dsm_trick_mode_flag,
            additional_copy_info_flag,
            pes_crc_flag,
            pes_extension_flag,
        )?;
        check_len!(bytes.len(), 3 + pes_header_data_length);
        let pes_packet_data_byte = &bytes[3 + pes_header_data_length..];
        Ok(NormalPESPacketBody {
            pes_scrambling_control,
            pes_priority,
            data_alignment_indicator,
            copyright,
            original_or_copy,
            pts,
            dts,
            escr,
            es_rate,
            dsm_trick_mode,
            additional_copy_info,
            previous_pes_packet_crc,
            pes_extension,
            pes_packet_data_byte,
        })
    }

    fn parse_optional_fields(
        mut bytes: &[u8],
        pts_dts_flags: u8,
        escr_flag: u8,
        es_rate_flag: u8,
        dsm_trick_mode_flag: u8,
        additional_copy_info_flag: u8,
        pes_crc_flag: u8,
        pes_extension_flag: u8,
    ) -> Result<
        (
            Option<u64>,
            Option<u64>,
            Option<ESCR>,
            Option<u32>,
            Option<DSMTrickMode>,
            Option<u8>,
            Option<u16>,
            Option<PESPacketExtension>,
        ),
        Error,
    > {
        let (pts, dts) = match pts_dts_flags {
            0b10 => {
                check_len!(bytes.len(), 5);
                let pts = NormalPESPacketBody::parse_timestamp(bytes)?;
                bytes = &bytes[5..];
                (Some(pts), None)
            }
            0b11 => {
                check_len!(bytes.len(), 10);
                let pts = NormalPESPacketBody::parse_timestamp(&bytes[0..])?;
                let dts = NormalPESPacketBody::parse_timestamp(&bytes[5..])?;
                bytes = &bytes[10..];
                (Some(pts), Some(dts))
            }
            _ => (None, None),
        };
        let escr = match escr_flag {
            1 => {
                check_len!(bytes.len(), 6);
                let escr = NormalPESPacketBody::parse_escr(bytes)?;
                bytes = &bytes[6..];
                Some(escr)
            }
            _ => None,
        };
        let es_rate = match es_rate_flag {
            1 => {
                check_len!(bytes.len(), 3);
                let es_rate = (u32::from(bytes[0] & 0x7f) << 15)
                    | (u32::from(bytes[1]) << 7)
                    | (u32::from(bytes[2]) >> 1);
                bytes = &bytes[3..];
                Some(es_rate)
            }
            _ => None,
        };
        let dsm_trick_mode = match dsm_trick_mode_flag {
            1 => {
                check_len!(bytes.len(), 1);
                // todo
                bytes = &bytes[1..];
                Some(DSMTrickMode {})
            }
            _ => None,
        };
        let additional_copy_info = match additional_copy_info_flag {
            1 => {
                check_len!(bytes.len(), 1);
                let additional_copy_info = bytes[0] & 0x7f;
                bytes = &bytes[1..];
                Some(additional_copy_info)
            }
            _ => None,
        };
        let previous_pes_packet_crc = match pes_crc_flag {
            1 => {
                check_len!(bytes.len(), 2);
                let previous_pes_packet_crc = (u16::from(bytes[0]) << 8) | u16::from(bytes[1]);
                bytes = &bytes[2..];
                Some(previous_pes_packet_crc)
            }
            _ => None,
        };
        let pes_extension = match pes_extension_flag {
            1 => Some(NormalPESPacketBody::parse_extension_fields(bytes)?),
            _ => None,
        };
        Ok((
            pts,
            dts,
            escr,
            es_rate,
            dsm_trick_mode,
            additional_copy_info,
            previous_pes_packet_crc,
            pes_extension,
        ))
    }

    fn parse_extension_fields(mut bytes: &[u8]) -> Result<PESPacketExtension, Error> {
        check_len!(bytes.len(), 1);
        let pes_private_data_flag = bytes[0] & 0x80 > 0;
        let pack_header_field_flag = bytes[0] & 0x40 > 0;
        let program_packet_sequence_counter_flag = bytes[0] & 0x20 > 0;
        let p_std_buffer_flag = bytes[0] & 0x10 > 0;
        let pes_extension_flag_2 = bytes[0] & 1 > 0;
        let pes_private_data = match pes_private_data_flag {
            true => {
                if bytes.len() < 16 {
                    bail!("too short for PES_private_data");
                }
                let pes_private_data = &bytes[..16];
                bytes = &bytes[16..];
                Some(pes_private_data)
            }
            _ => None,
        };
        let pack_header = if pack_header_field_flag {
            check_len!(bytes.len(), 1);
            let pack_field_length = usize::from(bytes[0]);
            check_len!(bytes.len(), 1 + pack_field_length);
            let pack_header = &bytes[1..1 + pack_field_length];
            bytes = &bytes[pack_field_length + 1..];
            Some(pack_header)
        } else {
            None
        };
        let (program_packet_sequence_counter, mpeg1_mpeg2_identifier, original_stuff_length) =
            match program_packet_sequence_counter_flag {
                true => {
                    check_len!(bytes.len(), 2);
                    let program_packet_sequence_counter = bytes[0] & 0x7f;
                    let mpeg1_mpeg2_identifier = (bytes[1] & 0x40) >> 6;
                    let original_stuff_length = bytes[1] & 0x3f;
                    bytes = &bytes[2..];
                    (
                        Some(program_packet_sequence_counter),
                        Some(mpeg1_mpeg2_identifier),
                        Some(original_stuff_length),
                    )
                }
                _ => (None, None, None),
            };
        let (p_std_buffer_scale, p_std_buffer_size) = match p_std_buffer_flag {
            true => {
                check_len!(bytes.len(), 2);
                let p_std_buffer_scale = (bytes[0] & 0x20) >> 5;
                let p_std_buffer_size = (u16::from(bytes[0] & 0x1f) << 8) | u16::from(bytes[1]);
                bytes = &bytes[2..];
                (Some(p_std_buffer_scale), Some(p_std_buffer_size))
            }
            _ => (None, None),
        };
        if pes_extension_flag_2 {
            check_len!(bytes.len(), 1);
            let _pes_extension_field_length = usize::from(bytes[0]) & 0x7f;
        }
        Ok(PESPacketExtension {
            pes_private_data,
            pack_header,
            program_packet_sequence_counter,
            mpeg1_mpeg2_identifier,
            original_stuff_length,
            p_std_buffer_scale,
            p_std_buffer_size,
        })
    }

    fn parse_timestamp(bytes: &[u8]) -> Result<u64, Error> {
        if bytes.len() < 5 {
            bail!("too short for timestamp {}", bytes.len());
        }
        Ok((u64::from(bytes[0] & 0xe) << 29)
            | (u64::from(bytes[1]) << 22)
            | (u64::from(bytes[2] & 0xfe) << 14)
            | (u64::from(bytes[3]) << 7)
            | (u64::from(bytes[4]) >> 1))
    }

    fn parse_escr(bytes: &[u8]) -> Result<ESCR, Error> {
        if bytes.len() < 6 {
            bail!("too short for ESCR");
        }
        let base = (u64::from(bytes[0] & 0x18) << 27)
            | (u64::from(bytes[0] & 0x3) << 28)
            | (u64::from(bytes[1]) << 20)
            | (u64::from(bytes[2] & 0xf8) << 12)
            | (u64::from(bytes[2] & 0x3) << 13)
            | (u64::from(bytes[3]) << 5)
            | (u64::from(bytes[4]) >> 3);
        let extension = (u16::from(bytes[4] & 0x3) << 7) | (u16::from(bytes[5]) >> 1);
        Ok(ESCR { base, extension })
    }
}
