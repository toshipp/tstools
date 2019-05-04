use env_logger;
use log::info;

use futures::future::lazy;

use serde_derive::Serialize;
use serde_json;
use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;
use tokio_channel::mpsc::{channel, Receiver, Sender};

use super::common;
use crate::pes;
use crate::psi;
use crate::ts;

#[derive(Serialize)]
struct Jitter {
    jitter: f64,
}

fn audio_processor(
    rx: Receiver<ts::TSPacket>,
    tx: Sender<u64>,
) -> impl Future<Item = (), Error = ()>
where
{
    pes::Buffer::new(rx)
        .and_then(|bytes| pes::PESPacket::parse(&bytes[..]).map(|pes| common::get_pts(&pes)))
        .filter_map(|x| x)
        .take(1)
        .forward(tx)
        .map(|_| ())
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
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

fn video_processor(
    rx: Receiver<ts::TSPacket>,
    tx: Sender<u64>,
) -> impl Future<Item = (), Error = ()> {
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
                                return common::get_pts(&pes);
                            }
                        }
                    }
                }
                None
            })
        })
        .filter_map(|x| x)
        .take(1)
        .forward(tx)
        .map(|_| ())
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

struct JitterProcessorSpawner {
    apts_tx: Sender<u64>,
    vpts_tx: Sender<u64>,
}

impl Clone for JitterProcessorSpawner {
    fn clone(&self) -> Self {
        JitterProcessorSpawner {
            apts_tx: self.apts_tx.clone(),
            vpts_tx: self.vpts_tx.clone(),
        }
    }
}

impl common::Spawner for JitterProcessorSpawner {
    fn spawn(
        &self,
        si: &psi::StreamInfo,
        demux_register: &mut ts::demuxer::Register,
    ) -> Result<(), ts::demuxer::RegistrationError> {
        if si.stream_type == psi::STREAM_TYPE_ADTS {
            match demux_register.try_register(si.elementary_pid) {
                Ok(rx) => {
                    tokio::spawn(audio_processor(rx, self.apts_tx.clone()));
                }
                Err(e) => {
                    if e.is_closed() {
                        return Err(e);
                    }
                }
            }
        }
        if si.stream_type == psi::STREAM_TYPE_VIDEO {
            match demux_register.try_register(si.elementary_pid) {
                Ok(rx) => {
                    tokio::spawn(video_processor(rx, self.vpts_tx.clone()));
                }
                Err(e) => {
                    if e.is_closed() {
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let (apts_tx, apts_rx) = channel(1);
        let (vpts_tx, vpts_rx) = channel(1);
        let demuxer = ts::demuxer::Demuxer::new();
        let spawner = JitterProcessorSpawner {
            apts_tx: apts_tx,
            vpts_tx: vpts_tx,
        };
        common::spawn_stream_splitter(spawner, demuxer.register());
        let decoder = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        apts_rx
            .zip(vpts_rx)
            .take(1)
            .for_each(|(apts, vpts)| {
                let jitter = Jitter {
                    jitter: f64::from((vpts - apts) as u32) / 90000f64,
                };
                println!("{}", serde_json::to_string(&jitter).unwrap());
                Ok(())
            })
            .select2(
                decoder
                    .forward(demuxer)
                    .map_err(|e| info!("decoding error: {}", e)),
            )
            .map(|_| ())
            .map_err(|_| ())
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
