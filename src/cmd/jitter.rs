use std::path::PathBuf;

use env_logger;
use log::info;

use futures::future::lazy;
use futures::sink::Sink;
use serde_derive::Serialize;
use serde_json;
use tokio::codec::FramedRead;
use tokio::fs::File;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Receiver, Sender};

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

struct FindPidSpawner {
    apid_tx: Option<Sender<u16>>,
    vpid_tx: Option<Sender<u16>>,
}

impl Clone for FindPidSpawner {
    fn clone(&self) -> Self {
        FindPidSpawner {
            apid_tx: self.apid_tx.clone(),
            vpid_tx: self.vpid_tx.clone(),
        }
    }
}

impl common::Spawner for FindPidSpawner {
    fn spawn(
        &mut self,
        si: &psi::StreamInfo,
        demux_register: &mut ts::demuxer::Register,
    ) -> Result<(), ts::demuxer::RegistrationError> {
        if si.stream_type == psi::STREAM_TYPE_ADTS {
            if let Some(tx) = self.apid_tx.take() {
                tokio::spawn(tx.send(si.elementary_pid).map(|_| ()).map_err(|_| ()));
            }
        }
        if si.stream_type == psi::STREAM_TYPE_VIDEO {
            if let Some(tx) = self.vpid_tx.take() {
                tokio::spawn(tx.send(si.elementary_pid).map(|_| ()).map_err(|_| ()));
            }
        }
        Ok(())
    }
}

fn find_pid(input: PathBuf) -> impl Future<Item = Option<(u16, u16)>, Error = ()> {
    lazy(move || {
        let (apid_tx, apid_rx) = channel(1);
        let (vpid_tx, vpid_rx) = channel(1);
        let demuxer = ts::demuxer::Demuxer::new();
        let spawner = FindPidSpawner {
            apid_tx: Some(apid_tx),
            vpid_tx: Some(vpid_tx),
        };
        common::spawn_stream_splitter(spawner, demuxer.register());
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

pub fn run(input: PathBuf) {
    env_logger::init();

    let proc = find_pid(input.clone())
        .map(|x| x.ok_or(()))
        .flatten()
        .and_then(|(apid, vpid)| {
            let (apts_tx, apts_rx) = channel(1);
            let (vpts_tx, vpts_rx) = channel(1);
            let demuxer = ts::demuxer::Demuxer::new();
            let mut register = demuxer.register();
            let rx = register.try_register(apid).unwrap();
            tokio::spawn(audio_processor(rx, apts_tx));
            let rx = register.try_register(vpid).unwrap();
            tokio::spawn(video_processor(rx, vpts_tx));
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
