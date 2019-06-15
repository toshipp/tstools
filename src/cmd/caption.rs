use failure::{bail, Error};

use env_logger;
use log::{debug, info};

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
use crate::stream::cueable;
use crate::ts;

fn sync_caption<'a>(pes: &'a pes::PESPacket) -> Result<arib::caption::DataGroup<'a>, Error> {
    if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
        arib::pes::SynchronizedPESData::parse(body.pes_packet_data_byte)
            .and_then(|data| arib::caption::DataGroup::parse(data.synchronized_pes_data_byte))
    } else {
        unreachable!();
    }
}

fn async_caption<'a>(pes: &'a pes::PESPacket) -> Result<arib::caption::DataGroup<'a>, Error> {
    if let pes::PESPacketBody::DataBytes(bytes) = pes.body {
        arib::pes::AsynchronousPESData::parse(bytes)
            .and_then(|data| arib::caption::DataGroup::parse(data.asynchronous_pes_data_byte))
    } else {
        unreachable!();
    }
}

fn get_caption<'a>(pes: &'a pes::PESPacket) -> Result<arib::caption::DataGroup<'a>, Error> {
    match pes.stream_id {
        arib::pes::SYNCHRONIZED_PES_STREAM_ID => sync_caption(pes),
        arib::pes::ASYNCHRONOUS_PES_STREAM_ID => async_caption(pes),
        _ => bail!("unknown pes"),
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
) -> Result<(), Error> {
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

fn process_captions<S: Stream<Item = ts::TSPacket, Error = Error>>(
    pid: u16,
    base_pts: u64,
    s: S,
) -> impl Future<Item = (), Error = Error> {
    let caption_stream = s.filter(move |packet| packet.pid == pid);
    pes::Buffer::new(caption_stream)
        .for_each(move |bytes| {
            let pes = match pes::PESPacket::parse(&bytes[..]) {
                Ok(pes) => pes,
                Err(e) => {
                    info!("pes parse error: {:?}", e);
                    return Ok(());
                }
            };
            let offset = match pes.get_pts() {
                Some(now) => now - base_pts,
                _ => return Ok(()),
            };
            let dg = match get_caption(&pes) {
                Ok(dg) => dg,
                Err(e) => {
                    info!("retrieving caption error: {:?}", e);
                    return Ok(());
                }
            };
            let data_units = match dg.data_group_data {
                arib::caption::DataGroupData::CaptionManagementData(ref cmd) => &cmd.data_units,
                arib::caption::DataGroupData::CaptionData(ref cd) => &cd.data_units,
            };
            if let Err(e) = dump_caption(data_units, offset) {
                info!("dump caption error: {:?}", e);
            }
            debug!("bytes: {:?}", bytes);
            Ok(())
        })
        .map(|_| ())
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
                    move |(pts, s)| {
                        let packets = s.cue_up();
                        process_captions(meta.caption_pid, pts, packets)
                    },
                )
            })
            .map_err(|e| info!("error: {}", e))
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
