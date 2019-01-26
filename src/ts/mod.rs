mod packet;
pub use self::packet::*;

pub mod demuxer;

pub const PAT_PID: u16 = 0;
pub const EIT_PIDS: [u16; 3] = [0x0012, 0x0026, 0x0027];
pub const CAT_PID: u16 = 1;
pub const TSDT_PID: u16 = 2;

pub const MPEG2_VIDEO_STREAM: u8 = 0x2;
pub const PES_PRIVATE_STREAM: u8 = 0x6;
pub const ADTS_AUDIO_STREAM: u8 = 0xf;
pub const H264_VIDEO_STREAM: u8 = 0x1b;
