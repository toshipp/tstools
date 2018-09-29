#[macro_use]
extern crate log;
extern crate env_logger;

#[macro_use]
extern crate failure;
use failure::Error;

#[macro_use]
extern crate lazy_static;

use std::collections::HashMap;
use std::io;
use std::io::{Error as StdError, ErrorKind, Read};

#[macro_use]
extern crate macros;

mod crc32;
mod pes;
mod psi;
mod ts;
mod util;

const PAT_PID: u16 = 0;
const CAT_PID: u16 = 1;
const TSDT_PID: u16 = 2;

const BS_sys: usize = 1536;
const TB_size: usize = 512;

const STREAM_TYPE_VIDEO: u8 = 0x2;
const STREAM_TYPE_PES_PRIVATE_DATA: u8 = 0x6;
const STREAM_TYPE_TYPE_D: u8 = 0xd;
const STREAM_TYPE_ADTS: u8 = 0xf;
const STREAM_TYPE_RESERVED_BEGIN: u8 = 0x15;
const STREAM_TYPE_RESERVED_END: u8 = 0x7f;
const STREAM_TYPE_H264: u8 = 0x1b;

#[derive(Debug)]
struct ProgramMapProcessor {
    pid: u16,
    started: bool,
    counter: u8,
    buffer: Vec<u8>,
}

impl ProgramMapProcessor {
    fn new(pid: u16) -> ProgramMapProcessor {
        return ProgramMapProcessor {
            pid,
            started: false,
            counter: 0,
            buffer: vec![],
        };
    }
    fn feed(&mut self, packet: &ts::TSPacket) -> Result<Option<psi::TSProgramMapSection>, Error> {
        let mut section = Ok(None);
        if packet.payload_unit_start_indicator {
            if self.started {
                section = psi::TSProgramMapSection::parse(self.buffer.as_slice()).map(|s| Some(s));
            }
            self.started = true;
            self.counter = packet.continuity_counter;
            let bytes = packet.data_byte.unwrap();
            let pointer_field = usize::from(bytes[0]);
            self.buffer.truncate(0);
            self.buffer.extend_from_slice(&bytes[1 + pointer_field..]);
        } else {
            if self.started {
                if self.counter == packet.continuity_counter {
                    // duplicate
                } else if ((self.counter + 1) % 16) == packet.continuity_counter {
                    self.counter = packet.continuity_counter;
                    self.buffer.extend_from_slice(packet.data_byte.unwrap());
                } else {
                    // discontinue, reset
                    self.started = false;
                }
            }
        }
        return section;
    }
}

struct TSPacketProcessor {
    program_map_processors: HashMap<u16, ProgramMapProcessor>,
    pes_processors: HashMap<u16, PESProcessor>,
}

impl TSPacketProcessor {
    fn new() -> TSPacketProcessor {
        TSPacketProcessor {
            program_map_processors: HashMap::new(),
            pes_processors: HashMap::new(),
        }
    }

    fn feed<R: Read>(&mut self, input: &mut R) -> Result<(), Error> {
        let mut buf = [0u8; ts::TS_PACKET_LENGTH];
        input.read_exact(&mut buf)?;
        let packet = ts::TSPacket::parse(&buf)?;

        if packet.transport_error_indicator {
            debug!("broken packet");
            return Ok(());
        }

        if packet.pid == PAT_PID {
            if packet.payload_unit_start_indicator {
                let data_byte = packet.data_byte.unwrap();
                let pointer_field = usize::from(data_byte[0]);
                let program_assoc_sec =
                    match psi::ProgramAssociationSection::parse(&data_byte[1 + pointer_field..]) {
                        Ok(sec) => sec,
                        Err(e) => {
                            debug!("raw: {:?}", &buf[..]);
                            debug!("packet: {:?}", packet);
                            return Err(e);
                        }
                    };
                for (program_number, pid) in program_assoc_sec.program_association {
                    if program_number != 0 {
                        // not network pid
                        self.program_map_processors
                            .entry(pid)
                            .or_insert(ProgramMapProcessor::new(pid));
                    }
                }
            }
        }
        if let Some(pmp) = self.program_map_processors.get_mut(&packet.pid) {
            match pmp.feed(&packet) {
                Ok(Some(pms)) => {
                    debug!("program map section: {:?}", pms);
                    for si in pms.stream_info.iter() {
                        // TODO
                        self.pes_processors
                            .entry(si.elementary_pid)
                            .or_insert(PESProcessor::new(si.stream_type, si.elementary_pid));
                    }
                }
                Err(e) => {
                    debug!("pmp: {:?}", pmp);
                    debug!("error: {:?}", e);
                    return Err(e);
                }
                _ => {}
            }
        }
        if let Some(pesp) = self.pes_processors.get_mut(&packet.pid) {
            match pesp.feed(&packet, |pes| {
                debug!("pes {:?}", pes);
                Ok(())
            }) {
                Err(e) => {
                    info!("error: {:?}", e);
                    info!("pesp {:?}", pesp);
                    return Err(e);
                }
                _ => {}
            };
        }
        Ok(())
    }
}

#[derive(Debug)]
struct PESProcessor {
    stream_type: u8,
    pid: u16,
    started: bool,
    counter: u8,
    buffer: Vec<u8>,
}

impl PESProcessor {
    fn new(stream_type: u8, pid: u16) -> PESProcessor {
        return PESProcessor {
            stream_type,
            pid,
            started: false,
            counter: 0,
            buffer: vec![],
        };
    }
    fn feed<F: Fn(pes::PESPacket) -> Result<(), Error>>(
        &mut self,
        packet: &ts::TSPacket,
        f: F,
    ) -> Result<(), Error> {
        let mut ret = Ok(());
        if packet.payload_unit_start_indicator {
            if self.started {
                ret = pes::PESPacket::parse(self.buffer.as_slice()).and_then(f);
            }
            let bytes = packet
                .data_byte
                .ok_or(format_err!("no data bytes packet"))?;
            self.started = true;
            self.counter = packet.continuity_counter;
            self.buffer.truncate(0);
            self.buffer.extend_from_slice(bytes);
        } else {
            if self.started {
                if self.counter == packet.continuity_counter {
                    // duplicate
                } else if ((self.counter + 1) % 16) == packet.continuity_counter {
                    let bytes = packet
                        .data_byte
                        .ok_or(format_err!("no data bytes packet"))?;
                    self.counter = packet.continuity_counter;
                    self.buffer.extend_from_slice(bytes);
                } else {
                    // discontinue, reset
                    self.started = false;
                }
            }
        }
        return ret;
    }
}

fn main() {
    env_logger::init();

    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut processor = TSPacketProcessor::new();
    loop {
        if let Err(e) = processor.feed(&mut handle) {
            if let Some(e) = e.root_cause().downcast_ref::<StdError>() {
                if e.kind() == ErrorKind::UnexpectedEof {
                    break;
                }
            }
            debug!("{:?}", e);
        }
    }
}
