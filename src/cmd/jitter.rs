use std::path::PathBuf;

use anyhow::{bail, Result};
use log::{info, warn};
use serde_derive::Serialize;
use serde_json;
use tokio_stream::{Stream, StreamExt};
use tokio_util::codec::FramedRead;

use super::common;
use super::io::path_to_async_read;
use crate::pes;
use crate::stream::cueable;
use crate::ts;

async fn find_first_audio_pts<S: Stream<Item = ts::TSPacket> + Unpin>(
    pid: u16,
    s: S,
) -> Result<u64> {
    let audio_stream = s.filter(move |packet| packet.pid == pid);
    let mut buffer = pes::Buffer::new(audio_stream);
    loop {
        match buffer.next().await {
            Some(Ok(bytes)) => {
                let pes = match pes::PESPacket::parse(&bytes[..]) {
                    Ok(pes) => pes,
                    Err(e) => {
                        warn!("pes parse error: {:?}", e);
                        continue;
                    }
                };
                if let Some(pts) = pes.get_pts() {
                    return Ok(pts);
                }
            }
            Some(Err(e)) => return Err(e),
            None => bail!("no pts found"),
        }
    }
}

#[derive(Serialize)]
struct Jitter {
    jitter: f64,
}

pub async fn run(input: Option<PathBuf>) -> Result<()> {
    let input = path_to_async_read(input).await?;
    let packets = FramedRead::new(input, ts::TSPacketDecoder::new());
    let packets = common::strip_error_packets(packets);
    let mut cueable_packets = cueable(packets);
    let meta = common::find_main_meta(&mut cueable_packets).await?;
    let packets = cueable_packets.cue_up();
    let mut cueable_packets = cueable(packets);
    let video_pts = common::find_first_picture_pts(meta.video_pid, &mut cueable_packets).await?;
    info!("video pts {}", video_pts);
    let packets = cueable_packets.cue_up();
    let audio_pts = find_first_audio_pts(meta.audio_pid, packets).await?;
    info!("audio pts {}", audio_pts);
    let jitter = Jitter {
        jitter: f64::from((video_pts - audio_pts) as u32) / 90000f64,
    };
    println!("{}", serde_json::to_string(&jitter)?);
    Ok(())
}
