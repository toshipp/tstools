mod packet;
pub use self::packet::*;

pub mod demuxer;

pub const PAT_PID: u16 = 0;
pub const EIT_PIDS: [u16; 3] = [0x0012, 0x0026, 0x0027];
#[allow(dead_code)]
pub const CAT_PID: u16 = 1;
#[allow(dead_code)]
pub const TSDT_PID: u16 = 2;
