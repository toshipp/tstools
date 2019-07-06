use std::path::PathBuf;

use failure::{bail, Error};
use futures::future::lazy;
use log::info;
use serde_derive::Serialize;
use serde_json;
use tokio::codec::FramedRead;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;

use super::common;
use super::io::path_to_async_read;
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

pub fn run(input: Option<PathBuf>) -> Result<(), Error> {
    let proc = lazy(|| {
        path_to_async_read(input).and_then(|input| {
            let packets = FramedRead::new(input, ts::TSPacketDecoder::new());
            let cueable_packets = cueable(packets);
            common::find_main_meta(cueable_packets).and_then(|(meta, s)| {
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
                            println!("{}", serde_json::to_string(&jitter)?);
                            Ok(())
                        })
                    },
                )
            })
        })
    });

    let rt = Builder::new().build()?;
    rt.block_on_all(proc)
}
