mod packet;
pub use self::packet::*;

pub mod demuxer;

pub const MPEG2_VIDEO_STREAM: u8 = 0x2;
pub const PES_PRIVATE_STREAM: u8 = 0x6;
pub const ADTS_AUDIO_STREAM: u8 = 0xf;
pub const H264_VIDEO_STREAM: u8 = 0x1b;
