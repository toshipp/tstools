use std::path::PathBuf;

use failure::{bail, Error};
use futures::future::lazy;
use log::{debug, info};
use serde_derive::Serialize;
use serde_json;
use tokio::codec::FramedRead;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;

use super::common;
use super::io::path_to_async_read;
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

fn log_drcs(mut b: &[u8]) {
    let number_of_code = b[0];
    info!("noc = {}", number_of_code);
    b = &b[1..];
    'outer: for _ in 0..number_of_code {
        let character_code = (u16::from(b[0]) << 8) | u16::from(b[1]);
        info!("cc = {}", character_code);
        let number_of_font = b[2];
        info!("nof {}", number_of_font);
        b = &b[3..];
        for _ in 0..number_of_font {
            let font_id = b[5] >> 4;
            let mode = b[5] & 0xf;
            info!("font_id = {} mode = {}", font_id, mode);
            b = &b[1..];
            if mode == 0 || mode == 1 {
                let depth = b[0];
                let width = b[1];
                let height = b[2];
                info!("d = {} w = {} h = {}", depth, width, height);
                let depth = u16::from(depth) + 1;
                let bits = 16 - depth.leading_zeros();
                let mask = (1 << bits) - 1;
                let mut pos = 0;
                let mut pbits = 0;
                b = &b[3..];
                for _ in 0..height {
                    let mut pic = String::new();
                    for _ in 0..width {
                        let v = (b[pos] >> (8 - bits - pbits)) & mask;
                        if v > 0 {
                            pic.push_str(&format!("{}", v));
                        } else {
                            pic.push(' ');
                        }
                        pbits += bits;
                        if pbits >= 8 {
                            pos += 1;
                            pbits -= 8;
                        }
                    }
                    info!("{:?}", pic);
                }
            } else {
                info!("geo")
            }
            break 'outer;
        }
    }
}

fn dump_caption<'a>(
    data_units: &Vec<arib::caption::DataUnit<'a>>,
    offset: u64,
) -> Result<(), Error> {
    for du in data_units {
        match &du.data_unit_parameter {
            arib::caption::DataUnitParameter::Text => {
                let decoder = arib::string::AribDecoder::with_caption_initialization();
                let caption_string = match decoder.decode(du.data_unit_data.iter()) {
                    Ok(s) => s,
                    Err(e) => {
                        debug!("raw: {:?}", du.data_unit_data);
                        return Err(e);
                    }
                };
                if !caption_string.is_empty() {
                    let caption = Caption {
                        time_sec: offset / pes::PTS_HZ,
                        time_ms: offset % pes::PTS_HZ * 1000 / pes::PTS_HZ,
                        caption: caption_string,
                    };
                    println!("{}", serde_json::to_string(&caption)?);
                }
            }
            arib::caption::DataUnitParameter::DRCS1 => {
                log_drcs(du.data_unit_data);
            }
            param => {
                debug!("unsupported data unit {:?}", param);
            }
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
                Some(now) => {
                    // if the caption is designated to be display
                    // before the first picture,
                    // ignore it.
                    if now < base_pts {
                        return Ok(());
                    }
                    now - base_pts
                }
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
            dump_caption(data_units, offset)?;
            Ok(())
        })
        .map(|_| ())
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
                    move |(pts, s)| {
                        let packets = s.cue_up();
                        process_captions(meta.caption_pid, pts, packets)
                    },
                )
            })
        })
    });

    let rt = Builder::new().build()?;
    rt.block_on_all(proc)
}
