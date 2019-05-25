use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use env_logger;
use log::{info, trace};

use futures::future::lazy;
use futures::sink::Sink;
use serde_derive::Serialize;
use serde_json;
use tokio::codec::FramedRead;
use tokio::fs::File;
use tokio::prelude::future::{Future, IntoFuture};
use tokio::prelude::Stream;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Sender};

use crate::pes;
use crate::psi;
use crate::ts;

struct FindPidSinkMaker {
    apid_tx: Sender<u16>,
    vpid_tx: Sender<u16>,
    pmt_pids: Arc<Mutex<HashSet<u16>>>,
}

impl FindPidSinkMaker {
    fn new(apid_tx: Sender<u16>, vpid_tx: Sender<u16>) -> Self {
        Self {
            apid_tx,
            vpid_tx,
            pmt_pids: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn make_pat_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let pmt_pids = self.pmt_pids.clone();
        tokio::spawn(
            psi::Buffer::new(rx)
                .take_while(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    if table_id == psi::PROGRAM_ASSOCIATION_SECTION {
                        let pas = match psi::ProgramAssociationSection::parse(bytes) {
                            Ok(pas) => pas,
                            Err(e) => {
                                info!("err {}: {:#?}", line!(), e);
                                return Ok(true);
                            }
                        };
                        for (program_number, pid) in pas.program_association {
                            if program_number != 0 {
                                // not network pid
                                let mut pmt_pids = pmt_pids.lock().unwrap();
                                pmt_pids.insert(pid);
                            }
                        }
                    }
                    Ok(true)
                })
                .for_each(|_| Ok(()))
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_pmt_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let apid_tx = self.apid_tx.clone();
        let vpid_tx = self.vpid_tx.clone();
        tokio::spawn(
            psi::Buffer::new(rx)
                .map(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    let mut apid = None;
                    let mut vpid = None;
                    if table_id == psi::TS_PROGRAM_MAP_SECTION {
                        match psi::TSProgramMapSection::parse(bytes) {
                            Ok(pms) => {
                                trace!("program map section: {:#?}", pms);
                                for si in pms.stream_info.iter() {
                                    if si.stream_type == psi::STREAM_TYPE_ADTS {
                                        apid = Some(si.elementary_pid);
                                    }
                                    if si.stream_type == psi::STREAM_TYPE_VIDEO {
                                        vpid = Some(si.elementary_pid);
                                    }
                                }
                            }
                            Err(e) => {
                                info!("err {}: {:#?}", line!(), e);
                            }
                        }
                    }
                    (apid, vpid)
                })
                .filter_map(|x| match x {
                    (Some(apid), Some(vpid)) => Some((apid, vpid)),
                    _ => None,
                })
                .for_each(move |(apid, vpid)| {
                    (apid_tx.clone().send(apid), vpid_tx.clone().send(vpid))
                        .into_future()
                        .map(|_| ())
                        .map_err(|e| e.into())
                })
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_sink(&mut self, pid: u16) -> Option<Sender<ts::TSPacket>> {
        if pid == 0 {
            return Some(self.make_pat_sink());
        }
        {
            let pmt_pids = self.pmt_pids.lock().unwrap();
            if pmt_pids.contains(&pid) {
                return Some(self.make_pmt_sink());
            }
        }
        None
    }
}

fn find_pid(input: PathBuf) -> impl Future<Item = Option<(u16, u16)>, Error = ()> {
    lazy(move || {
        let (apid_tx, apid_rx) = channel(1);
        let (vpid_tx, vpid_rx) = channel(1);
        let mut sink_maker = FindPidSinkMaker::new(apid_tx, vpid_tx);
        let demuxer = ts::demuxer::Demuxer::new(move |pid: u16| Ok(sink_maker.make_sink(pid)));
        File::open(input)
            .map_err(|e| info!("open error {}", e))
            .and_then(|file| {
                let decoder = FramedRead::new(file, ts::TSPacketDecoder::new());
                let pid_receive = apid_rx
                    .zip(vpid_rx)
                    .into_future()
                    .map(|(x, _)| x)
                    .map_err(|(e, _)| info!("recv err {:?}", e));

                let demux = decoder
                    .forward(demuxer)
                    .map(|_| ())
                    .map_err(|e| info!("decode err {:?}", e));
                tokio::spawn(demux);

                pid_receive
            })
    })
}

#[derive(Serialize)]
struct Jitter {
    jitter: f64,
}

const PICTURE_START_CODE: &[u8] = &[0, 0, 1, 0];
const I_PICTURE: u8 = 1;

fn index_pattern(pattern: &[u8], seq: &[u8]) -> Option<usize> {
    if pattern.len() > seq.len() {
        return None;
    }
    'outer: for i in 0..seq.len() - pattern.len() {
        for j in 0..pattern.len() {
            if seq[i + j] != pattern[j] {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

struct JitterSinkMaker {
    apid: u16,
    vpid: u16,
    apts_tx: Sender<u64>,
    vpts_tx: Sender<u64>,
}

impl JitterSinkMaker {
    fn new(apid: u16, vpid: u16, apts_tx: Sender<u64>, vpts_tx: Sender<u64>) -> Self {
        Self {
            apid,
            vpid,
            apts_tx,
            vpts_tx,
        }
    }

    fn make_video_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let vpts_tx = self.vpts_tx.clone();
        tokio::spawn(
            pes::Buffer::new(rx)
                .and_then(|bytes| {
                    pes::PESPacket::parse(&bytes[..]).map(|pes| {
                        if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
                            if let Some(index) =
                                index_pattern(PICTURE_START_CODE, body.pes_packet_data_byte)
                            {
                                let picture_header = &body.pes_packet_data_byte[index..];
                                if picture_header.len() >= 6 {
                                    let picture_coding_type = (picture_header[5] & 0x38) >> 3;
                                    if picture_coding_type == I_PICTURE {
                                        return pes.get_pts();
                                    }
                                }
                            }
                        }
                        None
                    })
                })
                .filter_map(|x| x)
                .take(1)
                .forward(vpts_tx)
                .map(|_| ())
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_audio_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let apts_tx = self.apts_tx.clone();
        tokio::spawn(
            pes::Buffer::new(rx)
                .and_then(|bytes| pes::PESPacket::parse(&bytes[..]).map(|pes| pes.get_pts()))
                .filter_map(|x| x)
                .take(1)
                .forward(apts_tx)
                .map(|_| ())
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_sink(&mut self, pid: u16) -> Option<Sender<ts::TSPacket>> {
        if self.vpid == pid {
            return Some(self.make_video_sink());
        }
        if self.apid == pid {
            return Some(self.make_audio_sink());
        }
        None
    }
}

pub fn run(input: PathBuf) {
    env_logger::init();

    let proc = find_pid(input.clone())
        .map(|x| x.ok_or(()))
        .flatten()
        .and_then(|(apid, vpid)| {
            let (apts_tx, apts_rx) = channel(1);
            let (vpts_tx, vpts_rx) = channel(1);
            let mut sink_maker = JitterSinkMaker::new(apid, vpid, apts_tx, vpts_tx);
            let demuxer = ts::demuxer::Demuxer::new(move |pid: u16| Ok(sink_maker.make_sink(pid)));
            info!("apid, vpid {}, {}", apid, vpid);

            File::open(input)
                .map_err(|e| info!("open error {}", e))
                .and_then(|file| {
                    let decoder = FramedRead::new(file, ts::TSPacketDecoder::new());
                    let demux = decoder
                        .forward(demuxer)
                        .map(|_| ())
                        .map_err(|e| info!("decode err {:?}", e));
                    tokio::spawn(demux);

                    apts_rx
                        .zip(vpts_rx)
                        .take(1)
                        .for_each(|(apts, vpts)| {
                            let jitter = Jitter {
                                jitter: f64::from((vpts - apts) as u32) / 90000f64,
                            };
                            info!("vpts {} apts {}", vpts, apts);
                            println!("{}", serde_json::to_string(&jitter).unwrap());
                            Ok(())
                        })
                        .map_err(|e| info!("{:?}", e))
                })
                .map(|_| ())
        });

    let mut rt = Builder::new().core_threads(2).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
