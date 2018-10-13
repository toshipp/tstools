#[macro_use]
extern crate log;
extern crate env_logger;

#[macro_use]
extern crate failure;
use failure::Error;

#[macro_use]
extern crate lazy_static;

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::io::{Error as StdError, ErrorKind, Read};
use std::mem;

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
struct PSIProcessor {
    pid: u16,
    table_id: u8,
    buffer: psi::Buffer,
}

impl PSIProcessor {
    fn new(pid: u16, table_id: u8) -> PSIProcessor {
        return PSIProcessor {
            pid,
            table_id,
            buffer: psi::Buffer::new(),
        };
    }
    fn feed<T, F: FnOnce(&[u8]) -> Result<T, Error>>(
        &mut self,
        packet: &ts::TSPacket,
        f: F,
    ) -> Result<Option<T>, Error> {
        match self.buffer.feed(packet)?.map(f) {
            Some(Ok(x)) => Ok(Some(x)),
            Some(Err(e)) => Err(e),
            _ => Ok(None),
        }
    }
}

struct TSPacketProcessor {
    psi_processors: HashMap<u16, PSIProcessor>,
    pes_processors: HashMap<u16, PESProcessor>,
    stream_types: HashSet<u8>,
}

impl TSPacketProcessor {
    fn new() -> TSPacketProcessor {
        let mut psip = HashMap::new();
        psip.insert(0, PSIProcessor::new(0, psi::PROGRAM_ASSOCIATION_SECTION));
        TSPacketProcessor {
            psi_processors: psip,
            pes_processors: HashMap::new(),
            stream_types: HashSet::new(),
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

        let stream_types = &mut mem::replace(&mut self.stream_types, HashSet::new());

        let ret = self.psi_processors.get_mut(&packet.pid).map(|processor| {
            processor.feed(&packet, |bytes| {
                let table_id = bytes[0];
                match table_id {
                    psi::PROGRAM_ASSOCIATION_SECTION => {
                        let pas = psi::ProgramAssociationSection::parse(bytes)?;
                        let mut procs = HashMap::new();
                        for (program_number, pid) in pas.program_association {
                            if program_number != 0 {
                                // not network pid
                                procs.insert(
                                    pid,
                                    PSIProcessor::new(pid, psi::TS_PROGRAM_MAP_SECTION),
                                );
                            }
                        }
                        return Ok((Some(procs), None));
                    }
                    psi::TS_PROGRAM_MAP_SECTION => {
                        let pms = psi::TSProgramMapSection::parse(bytes)?;
                        let mut procs = HashMap::new();
                        debug!("program map section: {:?}", pms);
                        for si in pms.stream_info.iter() {
                            // TODO
                            debug!("stream type: {}", si.stream_type);
                            stream_types.insert(si.stream_type);
                            procs.insert(
                                si.elementary_pid,
                                PESProcessor::new(si.stream_type, si.elementary_pid),
                            );
                        }
                        return Ok((None, Some(procs)));
                    }
                    _ => {
                        unreachable!("bug");
                    }
                }
            })
        });
        match ret {
            Some(Ok(Some((psi_procs, pes_procs)))) => {
                if let Some(mut psi_procs) = psi_procs {
                    for (pid, proc) in psi_procs.drain() {
                        self.psi_processors.entry(pid).or_insert(proc);
                    }
                }

                if let Some(mut pes_procs) = pes_procs {
                    for (pid, proc) in pes_procs.drain() {
                        self.pes_processors.entry(pid).or_insert(proc);
                    }
                }
            }
            Some(Err(e)) => {
                info!("psi process error: {:?}", e);
            }
            _ => {}
        };

        mem::swap(&mut self.stream_types, stream_types);

        if let Some(pesp) = self.pes_processors.get_mut(&packet.pid) {
            match pesp.feed(&packet, |pes| {
                debug!("pes {:?}", pes);
                Ok(())
            }) {
                Err(e) => {
                    info!("error: {:?}", e);
                    info!("pesp {:?}", pesp);
                    info!("packet {:?}", packet);
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
    buffer: psi::Buffer,
}

impl PESProcessor {
    fn new(stream_type: u8, pid: u16) -> PESProcessor {
        return PESProcessor {
            stream_type,
            pid,
            buffer: psi::Buffer::new(),
        };
    }
    fn feed<F: Fn(pes::PESPacket) -> Result<(), Error>>(
        &mut self,
        packet: &ts::TSPacket,
        f: F,
    ) -> Result<(), Error> {
        match self.buffer.feed(packet)?.map(|bytes| {
            let pes = pes::PESPacket::parse(bytes)?;
            f(pes)
        }) {
            Some(Err(e)) => Err(e),
            _ => Ok(()),
        }
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
    info!("types: {:?}", processor.stream_types);
}
