use failure;

use env_logger;
use log::{debug, info};

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;

use futures::future::lazy;

use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Sender};

use serde_derive::Serialize;
use serde_json;

use super::common;
use crate::arib;
use crate::pes;
use crate::psi;
use crate::ts;

fn is_caption_component(desc: &psi::Descriptor) -> bool {
    if let psi::Descriptor::StreamIdentifierDescriptor(sid) = desc {
        return arib::caption::is_non_partial_reception_caption(sid.component_tag);
    }
    false
}

fn is_caption(si: &psi::StreamInfo) -> bool {
    if si.stream_type == psi::STREAM_TYPE_PES_PRIVATE_DATA {
        return si.descriptors.iter().any(is_caption_component);
    }
    false
}

fn sync_caption<'a>(
    pes: &'a pes::PESPacket,
) -> Result<arib::caption::DataGroup<'a>, failure::Error> {
    if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
        arib::pes::SynchronizedPESData::parse(body.pes_packet_data_byte)
            .and_then(|data| arib::caption::DataGroup::parse(data.synchronized_pes_data_byte))
    } else {
        unreachable!();
    }
}

fn async_caption<'a>(
    pes: &'a pes::PESPacket,
) -> Result<arib::caption::DataGroup<'a>, failure::Error> {
    if let pes::PESPacketBody::DataBytes(bytes) = pes.body {
        arib::pes::AsynchronousPESData::parse(bytes)
            .and_then(|data| arib::caption::DataGroup::parse(data.asynchronous_pes_data_byte))
    } else {
        unreachable!();
    }
}

fn get_caption<'a>(
    pes: &'a pes::PESPacket,
) -> Result<Option<arib::caption::DataGroup<'a>>, failure::Error> {
    match pes.stream_id {
        arib::pes::SYNCHRONIZED_PES_STREAM_ID => sync_caption(pes).map(Some),
        arib::pes::ASYNCHRONOUS_PES_STREAM_ID => async_caption(pes).map(Some),
        _ => {
            info!("unknown pes");
            Ok(None)
        }
    }
}

#[derive(Serialize)]
struct Caption {
    time_sec: u64,
    time_ms: u64,
    caption: String,
}

fn dump_caption<'a>(
    data_units: &Vec<arib::caption::DataUnit<'a>>,
    offset: u64,
) -> Result<(), failure::Error> {
    for du in data_units {
        let caption_string = arib::string::decode_to_utf8(du.data_unit_data)?;
        if !caption_string.is_empty() {
            let caption = Caption {
                time_sec: offset / pes::PTS_HZ,
                time_ms: offset % pes::PTS_HZ * 1000 / pes::PTS_HZ,
                caption: caption_string,
            };
            println!("{}", serde_json::to_string(&caption).unwrap());
            debug!("raw {:?}", du.data_unit_data);
        }
    }
    Ok(())
}

struct SinkMaker {
    pmt_pids: Arc<Mutex<HashSet<u16>>>,
    video_pids: Arc<Mutex<HashSet<u16>>>,
    caption_pids: Arc<Mutex<HashSet<u16>>>,
    base_pts: Arc<Mutex<Option<u64>>>,
}

impl SinkMaker {
    fn new() -> Self {
        Self {
            pmt_pids: Arc::new(Mutex::new(HashSet::new())),
            video_pids: Arc::new(Mutex::new(HashSet::new())),
            caption_pids: Arc::new(Mutex::new(HashSet::new())),
            base_pts: Arc::new(Mutex::new(None)),
        }
    }

    fn make_pat_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let pmt_pids = self.pmt_pids.clone();
        tokio::spawn(
            psi::Buffer::new(rx)
                .for_each(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    if table_id == psi::PROGRAM_ASSOCIATION_SECTION {
                        let pas = match psi::ProgramAssociationSection::parse(bytes) {
                            Ok(pas) => pas,
                            Err(e) => {
                                info!("err {}: {:#?}", line!(), e);
                                return Ok(());
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
                    Ok(())
                })
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_pmt_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let caption_pids = self.caption_pids.clone();
        let video_pids = self.video_pids.clone();
        tokio::spawn(
            psi::Buffer::new(rx)
                .for_each(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    if table_id == psi::TS_PROGRAM_MAP_SECTION {
                        let pms = match psi::TSProgramMapSection::parse(bytes) {
                            Ok(pms) => pms,
                            Err(e) => {
                                info!("err {}: {:#?}", line!(), e);
                                return Ok(());
                            }
                        };
                        for si in pms.stream_info.iter() {
                            if is_caption(&si) {
                                let mut caption_pids = caption_pids.lock().unwrap();
                                caption_pids.insert(si.elementary_pid);
                            }
                            if si.stream_type == psi::STREAM_TYPE_VIDEO {
                                let mut video_pids = video_pids.lock().unwrap();
                                video_pids.insert(si.elementary_pid);
                            }
                        }
                    }
                    Ok(())
                })
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_video_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let base_pts = self.base_pts.clone();
        tokio::spawn(
            pes::Buffer::new(rx)
                .for_each(move |bytes| {
                    let mut base_pts = base_pts.lock().unwrap();
                    if base_pts.is_some() {
                        return Ok(());
                    }
                    pes::PESPacket::parse(&bytes[..]).and_then(|pes| {
                        if let Some(pts) = common::get_pts(&pes) {
                            *base_pts = dbg!(Some(pts));
                        }
                        Ok(())
                    })
                })
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_caption_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let base_pts = self.base_pts.clone();
        tokio::spawn(
            pes::Buffer::new(rx)
                .for_each(move |bytes| {
                    pes::PESPacket::parse(&bytes[..]).and_then(|pes| {
                        let base_pts = base_pts.lock().unwrap();
                        let offset = match (common::get_pts(&pes), *base_pts) {
                            (Some(now), Some(base)) => now - base,
                            _ => return Ok(()),
                        };
                        if let Some(dg) = get_caption(&pes)? {
                            let data_units = match dg.data_group_data {
                                arib::caption::DataGroupData::CaptionManagementData(ref cmd) => {
                                    &cmd.data_units
                                }
                                arib::caption::DataGroupData::CaptionData(ref cd) => &cd.data_units,
                            };
                            dump_caption(data_units, offset)?;
                            debug!("bytes: {:?}", bytes);
                        }
                        Ok(())
                    })
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
        {
            let video_pids = self.video_pids.lock().unwrap();
            if video_pids.contains(&pid) {
                return Some(self.make_video_sink());
            }
        }
        {
            let caption_pids = self.caption_pids.lock().unwrap();
            if caption_pids.contains(&pid) {
                return Some(self.make_caption_sink());
            }
        }
        None
    }
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let mut sink_maker = SinkMaker::new();
        let demuxer = ts::demuxer::Demuxer::new(move |pid: u16| Ok(sink_maker.make_sink(pid)));

        let decoder = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        decoder
            .forward(demuxer)
            .map(|_| ())
            .map_err(|e| info!("error: {}", e))
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
