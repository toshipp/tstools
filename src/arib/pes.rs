use anyhow::Result;

pub const SYNCHRONIZED_PES_STREAM_ID: u8 = 0xbd;
pub const ASYNCHRONOUS_PES_STREAM_ID: u8 = 0xbf;

pub struct SynchronizedPESData<'a> {
    pub data_identifier: u8,
    pub private_stream_id: u8,
    pub pes_data_private_data_byte: &'a [u8],
    pub synchronized_pes_data_byte: &'a [u8],
}

impl<'a> SynchronizedPESData<'a> {
    pub fn parse(bytes: &[u8]) -> Result<SynchronizedPESData> {
        let data_identifier = bytes[0];
        let private_stream_id = bytes[1];
        let pes_data_packet_header_length = usize::from(bytes[2] & 0xf);
        let pes_data_private_data_byte = &bytes[3..3 + pes_data_packet_header_length];
        let synchronized_pes_data_byte = &bytes[3 + pes_data_packet_header_length..];
        Ok(SynchronizedPESData {
            data_identifier,
            private_stream_id,
            pes_data_private_data_byte,
            synchronized_pes_data_byte,
        })
    }
}

pub struct AsynchronousPESData<'a> {
    pub data_identifier: u8,
    pub private_stream_id: u8,
    pub pes_data_private_data_byte: &'a [u8],
    pub asynchronous_pes_data_byte: &'a [u8],
}

impl<'a> AsynchronousPESData<'a> {
    pub fn parse(bytes: &[u8]) -> Result<AsynchronousPESData> {
        let data_identifier = bytes[0];
        let private_stream_id = bytes[1];
        let pes_data_packet_header_length = usize::from(bytes[2] & 0xf);
        let pes_data_private_data_byte = &bytes[3..3 + pes_data_packet_header_length];
        let asynchronous_pes_data_byte = &bytes[3 + pes_data_packet_header_length..];
        Ok(AsynchronousPESData {
            data_identifier,
            private_stream_id,
            pes_data_private_data_byte,
            asynchronous_pes_data_byte,
        })
    }
}
