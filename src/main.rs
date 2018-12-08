#[macro_use]
extern crate log;
use env_logger;

#[macro_use]
extern crate failure;
use failure::Error;

#[macro_use]
extern crate lazy_static;

use std::collections::HashMap;
use std::collections::HashSet;
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
#[allow(dead_code)]
const CAT_PID: u16 = 1;
#[allow(dead_code)]
const TSDT_PID: u16 = 2;

const EIT_PIDS: [u16; 3] = [0x0012, 0x0026, 0x0027];

#[allow(dead_code)]
const BS_SYS: usize = 1536;
#[allow(dead_code)]
const TB_SIZE: usize = 512;

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

#[derive(Debug)]
struct PSIProcessor {
    pid: u16,
    buffer: psi::Buffer,
}

impl PSIProcessor {
    fn new(pid: u16) -> PSIProcessor {
        return PSIProcessor {
            pid,
            buffer: psi::Buffer::new(),
        };
    }
    fn feed<T, F: FnOnce(&[u8]) -> Result<T, Error>>(
        &mut self,
        packet: &ts::TSPacket<'_>,
        f: F,
    ) -> Result<Option<T>, Error> {
        match self.buffer.feed(packet)?.map(f) {
            Some(Ok(x)) => Ok(Some(x)),
            Some(Err(e)) => Err(e),
            _ => Ok(None),
        }
    }
    #[allow(dead_code)]
    fn get_buffer(&self) -> &psi::Buffer {
        &self.buffer
    }
}

struct TSPacketProcessor {
    psi_processors: HashMap<u16, PSIProcessor>,
    pes_processors: HashMap<u16, PESProcessor>,
    stream_types: HashSet<u8>,
    descriptors: HashSet<u8>,
    pids: HashSet<u16>,
}

impl TSPacketProcessor {
    fn new() -> TSPacketProcessor {
        let mut psip = HashMap::new();
        psip.insert(PAT_PID, PSIProcessor::new(PAT_PID));
        for pid in EIT_PIDS.iter() {
            psip.insert(*pid, PSIProcessor::new(*pid));
        }
        TSPacketProcessor {
            psi_processors: psip,
            pes_processors: HashMap::new(),
            stream_types: HashSet::new(),
            descriptors: HashSet::new(),
            pids: HashSet::new(),
        }
    }

    fn process_psi(&mut self, packet: &ts::TSPacket<'_>) -> Result<(), Error> {
        let mut stream_types = HashSet::new();
        let mut psi_procs = Vec::new();
        let mut pes_procs = Vec::new();
        let descriptors = Vec::new();

        if let Some(proc) = self.psi_processors.get_mut(&packet.pid) {
            match proc.feed(&packet, |bytes| {
                let table_id = bytes[0];
                match table_id {
                    psi::PROGRAM_ASSOCIATION_SECTION => {
                        let pas = psi::ProgramAssociationSection::parse(bytes)?;
                        for (program_number, pid) in pas.program_association {
                            if program_number != 0 {
                                // not network pid
                                psi_procs.push((pid, PSIProcessor::new(pid)));
                            }
                        }
                        return Ok(());
                    }
                    psi::TS_PROGRAM_MAP_SECTION => {
                        let pms = psi::TSProgramMapSection::parse(bytes)?;
                        debug!("program map section: {:?}", pms);
                        for si in pms.stream_info.iter() {
                            stream_types.insert(si.stream_type);
                            match si.stream_type {
                                ts::MPEG2_VIDEO_STREAM
                                | ts::PES_PRIVATE_STREAM
                                | ts::ADTS_AUDIO_STREAM
                                | ts::H264_VIDEO_STREAM => pes_procs.push((
                                    si.elementary_pid,
                                    PESProcessor::new(si.stream_type, si.elementary_pid),
                                )),
                                _ => {}
                            };
                        }
                        return Ok(());
                    }
                    n if 0x4e <= n && n <= 0x6f => {
                        let eit = psi::EventInformationSection::parse(bytes)?;
                        info!("pid: {}, eit: {:?}", packet.pid, eit);
                        return Ok(());
                    }
                    _ => {
                        unreachable!("bug");
                    }
                }
            }) {
                Err(e) => {
                    info!("psi process error: {:?}", e);
                    return Err(e);
                }
                _ => {}
            }
        }

        for (pid, proc) in psi_procs.into_iter() {
            self.psi_processors.entry(pid).or_insert(proc);
        }
        for (pid, proc) in pes_procs.into_iter() {
            self.pes_processors.entry(pid).or_insert(proc);
        }
        self.stream_types.extend(stream_types.iter());
        self.descriptors.extend(descriptors.iter());

        Ok(())
    }

    fn process_pes(&mut self, packet: &ts::TSPacket<'_>) -> Result<(), Error> {
        if let Some(proc) = self.pes_processors.get_mut(&packet.pid) {
            match proc.feed(&packet, |pes| {
                if pes.stream_id == 0b10111101 {
                    // info!("pes private stream1 {:?}", pes);
                } else {
                    debug!("pes {:?}", pes);
                }
                Ok(())
            }) {
                Err(e) => {
                    info!("error: {:?}", e);
                    info!("pesp {:?}", proc);
                    info!("packet {:?}", packet);
                    return Err(e);
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn feed<R: Read>(&mut self, input: &mut R) -> Result<(), Error> {
        let mut buf = [0u8; ts::TS_PACKET_LENGTH];
        input.read_exact(&mut buf)?;
        let packet = ts::TSPacket::parse(&buf)?;

        if packet.transport_error_indicator {
            debug!("broken packet");
            return Ok(());
        }

        self.process_psi(&packet)?;
        self.process_pes(&packet)?;

        self.pids.insert(packet.pid);

        Ok(())
    }
}

#[derive(Debug)]
struct PESProcessor {
    stream_type: u8,
    pid: u16,
    buffer: pes::Buffer,
}

impl PESProcessor {
    fn new(stream_type: u8, pid: u16) -> PESProcessor {
        return PESProcessor {
            stream_type,
            pid,
            buffer: pes::Buffer::new(),
        };
    }
    fn feed<F: FnMut(pes::PESPacket<'_>) -> Result<(), Error>>(
        &mut self,
        packet: &ts::TSPacket<'_>,
        mut f: F,
    ) -> Result<(), Error> {
        self.buffer
            .feed(packet, |bytes| match pes::PESPacket::parse(bytes) {
                Ok(pes) => f(pes),
                Err(e) => {
                    info!("pes parse error raw bytes : {:?}", bytes);
                    Err(e)
                }
            })
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
    info!("descriptors: {:?}", processor.descriptors);
    let pids = processor
        .pes_processors
        .keys()
        .chain(processor.psi_processors.keys())
        .map(|x| *x)
        .collect::<HashSet<_>>();
    info!("proceeded {:?}", pids);
    info!("pids: {:?}", processor.pids.difference(&pids));
}
