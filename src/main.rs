#[macro_use]
mod util;
mod arib;
mod cmd;
mod crc32;
mod pes;
mod psi;
mod ts;

#[allow(dead_code)]
const STREAM_TYPE_VIDEO: u8 = 0x2;
#[allow(dead_code)]
const STREAM_TYPE_PES_PRIVATE_DATA: u8 = 0x6;
#[allow(dead_code)]
const STREAM_TYPE_TYPE_D: u8 = 0xd;
#[allow(dead_code)]
const STREAM_TYPE_ADTS: u8 = 0xf;
#[allow(dead_code)]
const STREAM_TYPE_RESERVED_BEGIN: u8 = 0x15;
#[allow(dead_code)]
const STREAM_TYPE_RESERVED_END: u8 = 0x7f;
#[allow(dead_code)]
const STREAM_TYPE_H264: u8 = 0x1b;

fn main() {
    cmd::dump_program::run();
}
