use env_logger;
use log::{debug, info};

use std::sync::Arc;
use std::sync::Mutex;

use std::fmt::Debug;

use std::collections::HashSet;

use futures::future::lazy;

use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;

use crate::pes;
use crate::psi;
use crate::ts;

struct Context {
    stream_types: HashSet<u8>,
    descriptors: HashSet<u8>,
}

impl Context {
    fn new() -> Context {
        Context {
            stream_types: HashSet::new(),
            descriptors: HashSet::new(),
        }
    }
}

fn pat_processor<S, E>(
    pctx: Arc<Mutex<Context>>,
    mut demux_register: ts::demuxer::Register,
    s: S,
) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s)
        .for_each(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            match table_id {
                psi::PROGRAM_ASSOCIATION_SECTION => {
                    let pas = psi::ProgramAssociationSection::parse(bytes)?;
                    for (program_number, pid) in pas.program_association {
                        if program_number != 0 {
                            // not network pid
                            if let Ok(rx) = demux_register.try_register(pid) {
                                tokio::spawn(pmt_processor(
                                    pctx.clone(),
                                    demux_register.clone(),
                                    rx,
                                ));
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

fn pmt_processor<S, E>(
    pctx: Arc<Mutex<Context>>,
    mut demux_regiser: ts::demuxer::Register,
    s: S,
) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s)
        .for_each(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            match table_id {
                psi::TS_PROGRAM_MAP_SECTION => {
                    let mut ctx = pctx.lock().unwrap();
                    let pms = psi::TSProgramMapSection::parse(bytes)?;
                    debug!("program map section: {:#?}", pms);
                    for si in pms.stream_info.iter() {
                        ctx.stream_types.insert(si.stream_type);
                        match si.stream_type {
                            ts::MPEG2_VIDEO_STREAM
                            | ts::PES_PRIVATE_STREAM
                            | ts::ADTS_AUDIO_STREAM
                            | ts::H264_VIDEO_STREAM => {
                                if let Ok(rx) = demux_regiser.try_register(si.elementary_pid) {
                                    tokio::spawn(pes_processor(rx));
                                }
                            }
                            _ => {}
                        };
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

fn pes_processor<S, E>(s: S) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    pes::Buffer::new(s)
        .for_each(move |bytes| {
            match pes::PESPacket::parse(&bytes[..]) {
                Ok(pes) => {
                    if pes.stream_id == 0b10111101 {
                        // info!("pes private stream1 {:?}", pes);
                    } else {
                        debug!("pes {:#?}", pes);
                    }
                }
                Err(e) => {
                    info!("pes parse error: {:#?}", e);
                    info!("raw bytes : {:#?}", bytes);
                }
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let pctx = Arc::new(Mutex::new(Context::new()));
        let demuxer = ts::demuxer::Demuxer::new();
        let mut demux_register = demuxer.register();
        // pat
        tokio::spawn(pat_processor(
            pctx.clone(),
            demux_register.clone(),
            demux_register.try_register(ts::PAT_PID).unwrap(),
        ));

        let decoder = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        decoder.forward(demuxer).then(move |ret| {
            if let Err(e) = ret {
                info!("err: {}", e);
            }
            let ctx = pctx.lock().unwrap();
            info!("types: {:#?}", ctx.stream_types);
            info!("descriptors: {:#?}", ctx.descriptors);
            Ok(())
        })
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
