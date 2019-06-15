use env_logger;
use log::info;

use failure::{bail, Error};
use futures::future::lazy;

use serde_derive::Serialize;
use serde_json;
use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;

use super::common;
use crate::pes;

use crate::stream::cueable;
use crate::ts;

fn find_first_audio_pts<S: Stream<Item = ts::TSPacket, Error = Error>>(
    pid: u16,
    s: S,
) -> impl Future<Item = u64, Error = Error> {
    let audio_stream = s.filter(move |packet| packet.pid == pid);
    pes::Buffer::new(audio_stream)
        .filter_map(|bytes| {
            let pes = match pes::PESPacket::parse(&bytes[..]) {
                Ok(pes) => pes,
                Err(e) => {
                    info!("pes parse error: {:?}", e);
                    return None;
                }
            };
            pes.get_pts()
        })
        .into_future()
        .map_err(|(e, _)| e)
        .and_then(|(pts, _)| match pts {
            Some(pts) => Ok(pts),
            None => bail!("no pts found"),
        })
}

#[derive(Serialize)]
struct Jitter {
    jitter: f64,
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let packets = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        let cueable_packets = cueable(packets);
        common::find_main_meta(cueable_packets)
            .and_then(|(meta, s)| {
                let packets = s.cue_up();
                let cueable_packets = cueable(packets);
                common::find_first_picture_pts(meta.video_pid, cueable_packets).and_then(
                    move |(video_pts, s)| {
                        let packets = s.cue_up();
                        find_first_audio_pts(meta.audio_pid, packets).and_then(move |audio_pts| {
                            let jitter = Jitter {
                                jitter: f64::from((video_pts - audio_pts) as u32) / 90000f64,
                            };
                            info!("vpts {} apts {}", video_pts, audio_pts);
                            println!("{}", serde_json::to_string(&jitter).unwrap());
                            Ok(())
                        })
                    },
                )
            })
            .map_err(|e| info!("error: {}", e))
    });

    let mut rt = Builder::new().core_threads(2).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
