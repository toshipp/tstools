use failure;

use env_logger;
use log::{debug, info};

use std::sync::Arc;
use std::sync::Mutex;

use std::fmt::Debug;

use futures::future::lazy;

use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;

use serde_derive::Serialize;
use serde_json;

use super::common;
use crate::arib;
use crate::pes;
use crate::psi;
use crate::ts;

struct Context {
    base_pts: Option<u64>,
}

impl Context {
    fn new() -> Context {
        Context { base_pts: None }
    }
}

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

fn video_pts_processor<S, E>(pctx: Arc<Mutex<Context>>, s: S) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    pes::Buffer::new(s)
        .for_each(move |bytes| {
            let mut ctx = pctx.lock().unwrap();
            if ctx.base_pts.is_some() {
                return Ok(());
            }
            pes::PESPacket::parse(&bytes[..]).and_then(|pes| {
                if let Some(pts) = common::get_pts(&pes) {
                    ctx.base_pts = Some(pts);
                }
                Ok(())
            })
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

fn caption_processor<S, E>(pctx: Arc<Mutex<Context>>, s: S) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    pes::Buffer::new(s)
        .for_each(move |bytes| {
            pes::PESPacket::parse(&bytes[..]).and_then(|pes| {
                let ctx = pctx.lock().unwrap();
                let offset = match (common::get_pts(&pes), ctx.base_pts) {
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
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

struct CaptionProcessorSpawner {
    pctx: Arc<Mutex<Context>>,
}

impl Clone for CaptionProcessorSpawner {
    fn clone(&self) -> Self {
        CaptionProcessorSpawner {
            pctx: self.pctx.clone(),
        }
    }
}

impl common::Spawner for CaptionProcessorSpawner {
    fn spawn(&self, si: &psi::StreamInfo, demux_regiser: &mut ts::demuxer::Register) {
        if is_caption(&si) {
            if let Ok(rx) = demux_regiser.try_register(si.elementary_pid) {
                tokio::spawn(caption_processor(self.pctx.clone(), rx));
            }
        }
        if si.stream_type == psi::STREAM_TYPE_VIDEO {
            if let Ok(rx) = demux_regiser.try_register(si.elementary_pid) {
                tokio::spawn(video_pts_processor(self.pctx.clone(), rx));
            }
        }
    }
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let pctx = Arc::new(Mutex::new(Context::new()));
        let demuxer = ts::demuxer::Demuxer::new();
        common::spawn_stream_splitter(CaptionProcessorSpawner { pctx: pctx }, demuxer.register());
        let decoder = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        decoder.forward(demuxer).then(move |ret| {
            if let Err(e) = ret {
                info!("err: {}", e);
            }
            Ok(())
        })
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
