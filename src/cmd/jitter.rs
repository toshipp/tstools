use env_logger;
use log::info;

use std::sync::Arc;
use std::sync::Mutex;

use futures::future::lazy;

use serde_derive::Serialize;
use serde_json;
use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;
use tokio_channel::mpsc::Receiver;

use super::common;
use crate::pes;
use crate::psi;
use crate::ts;

#[derive(Serialize)]
struct Jitter {
    jitter: f64,
}

struct Context {
    audio_pts: Option<u64>,
    video_pts: Option<u64>,
}

fn audio_processor(
    pctx: Arc<Mutex<Context>>,
    rx: Receiver<ts::TSPacket>,
) -> impl Future<Item = (), Error = ()>
where
{
    pes::Buffer::new(rx)
        .for_each(move |bytes| {
            pes::PESPacket::parse(&bytes[..]).and_then(|pes| {
                let mut ctx = pctx.lock().unwrap();
                if ctx.audio_pts.is_none() {
                    if let Some(pts) = common::get_pts(&pes) {
                        ctx.audio_pts = Some(pts);
                    }
                }
                Ok(())
            })
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

const PICTURE_START_CODE: &[u8] = &[0, 0, 1, 0];

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

fn video_processor(
    pctx: Arc<Mutex<Context>>,
    rx: Receiver<ts::TSPacket>,
) -> impl Future<Item = (), Error = ()> {
    pes::Buffer::new(rx)
        .for_each(move |bytes| {
            pes::PESPacket::parse(&bytes[..]).and_then(|pes| {
                let mut ctx = pctx.lock().unwrap();
                if ctx.video_pts.is_some() {
                    return Ok(());
                }
                if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
                    if let Some(index) =
                        index_pattern(PICTURE_START_CODE, body.pes_packet_data_byte)
                    {
                        let picture_header = &body.pes_packet_data_byte[index..];
                        if picture_header.len() < 6 {
                            return Ok(());
                        }
                        let picture_coding_type = (picture_header[5] & 0x38) >> 3;
                        // I picture
                        if picture_coding_type == 1 {
                            ctx.video_pts = dbg!(common::get_pts(&pes));
                        }
                    }
                }
                Ok(())
            })
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

struct JitterProcessorSpawner {
    pctx: Arc<Mutex<Context>>,
}

impl Clone for JitterProcessorSpawner {
    fn clone(&self) -> Self {
        JitterProcessorSpawner {
            pctx: self.pctx.clone(),
        }
    }
}

impl common::Spawner for JitterProcessorSpawner {
    fn spawn(&self, si: &psi::StreamInfo, demux_register: &mut ts::demuxer::Register) {
        if si.stream_type == psi::STREAM_TYPE_ADTS {
            if let Ok(rx) = demux_register.try_register(si.elementary_pid) {
                tokio::spawn(audio_processor(self.pctx.clone(), rx));
            }
        }
        if si.stream_type == psi::STREAM_TYPE_VIDEO {
            if let Ok(rx) = demux_register.try_register(si.elementary_pid) {
                tokio::spawn(video_processor(self.pctx.clone(), rx));
            }
        }
    }
}

pub fn run() {
    env_logger::init();

    let pctx = Arc::new(Mutex::new(Context {
        audio_pts: None,
        video_pts: None,
    }));
    let pctx2 = pctx.clone();
    let proc = lazy(|| {
        let demuxer = ts::demuxer::Demuxer::new();
        common::spawn_stream_splitter(JitterProcessorSpawner { pctx: pctx2 }, demuxer.register());
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

    let ctx = pctx.lock().unwrap();
    match (ctx.audio_pts, ctx.video_pts) {
        (Some(a), Some(v)) => {
            let jitter = Jitter {
                jitter: f64::from((a - v) as u32) / 90000f64,
            };
            println!("{}", serde_json::to_string(&jitter).unwrap());
        }
        _ => {}
    }
}
