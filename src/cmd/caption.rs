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

use crate::arib;
use crate::h262;
use crate::pes;
use crate::psi;
use crate::stream::cueable;
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

fn find_main_pmt_pid<S: Stream<Item = ts::TSPacket, Error = Error>>(
    s: S,
) -> impl Future<Item = (u16, S), Error = Error> {
    let pat_stream = s.filter(|packet| packet.pid == ts::PAT_PID);
    psi::Buffer::new(pat_stream)
        .filter_map(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            if table_id == psi::PROGRAM_ASSOCIATION_SECTION {
                let pas = match psi::ProgramAssociationSection::parse(bytes) {
                    Ok(pas) => pas,
                    Err(e) => {
                        info!("pat parse error: {:?}", e);
                        return None;
                    }
                };
                for (program_number, pid) in pas.program_association {
                    if program_number != 0 {
                        // not network pid
                        return Some(pid);
                    }
                }
            }
            None
        })
        .into_future()
        .map(|(pids, stream)| (pids, stream.into_inner().into_inner().into_inner()))
        .map_err(|(e, _)| e)
        .and_then(|(pids, s)| match pids {
            Some(pids) => Ok((pids, s)),
            None => bail!("no pid found"),
        })
}

struct Meta {
    audio_pid: u16,
    video_pid: u16,
    caption_pid: u16,
}

fn find_meta<S: Stream<Item = ts::TSPacket, Error = Error>>(
    pid: u16,
    s: S,
) -> impl Future<Item = (Meta, S), Error = Error> {
    let pmt_stream = s.filter(move |packet| packet.pid == pid);
    psi::Buffer::new(pmt_stream)
        .filter_map(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            if table_id == psi::TS_PROGRAM_MAP_SECTION {
                let pms = match psi::TSProgramMapSection::parse(bytes) {
                    Ok(pms) => pms,
                    Err(e) => {
                        info!("pmt parse error: {:?}", e);
                        return None;
                    }
                };
                let mut video_pid = None;
                let mut audio_pid = None;
                let mut caption_pid = None;
                for si in pms.stream_info.iter() {
                    if caption_pid.is_none() && is_caption(&si) {
                        caption_pid = Some(si.elementary_pid);
                    }
                    if video_pid.is_none() && si.stream_type == psi::STREAM_TYPE_VIDEO {
                        video_pid = Some(si.elementary_pid);
                    }
                    if audio_pid.is_none() && si.stream_type == psi::STREAM_TYPE_ADTS {
                        audio_pid = Some(si.elementary_pid);
                    }
                }
                match (video_pid, audio_pid, caption_pid) {
                    (Some(video_pid), Some(audio_pid), Some(caption_pid)) => {
                        return Some(Meta {
                            audio_pid,
                            video_pid,
                            caption_pid,
                        })
                    }
                    _ => {}
                }
            }
            None
        })
        .into_future()
        .map_err(|(e, _)| e)
        .and_then(|(meta, s)| match meta {
            Some(meta) => Ok((meta, s.into_inner().into_inner().into_inner())),
            None => bail!("no meta found"),
        })
}

fn find_first_picture_pts<S: Stream<Item = ts::TSPacket, Error = Error>>(
    pid: u16,
    s: S,
) -> impl Future<Item = (u64, S), Error = Error> {
    let video_stream = s.filter(move |packet| packet.pid == pid);
    pes::Buffer::new(video_stream)
        .filter_map(|bytes| {
            let pes = match pes::PESPacket::parse(&bytes[..]) {
                Ok(pes) => pes,
                Err(e) => {
                    info!("pes parse error: {:?}", e);
                    return None;
                }
            };
            if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
                if h262::is_i_picture(body.pes_packet_data_byte) {
                    return pes.get_pts();
                }
            }
            None
        })
        .into_future()
        .map_err(|(e, _)| e)
        .and_then(|(pts, s)| match pts {
            Some(pts) => Ok((pts, s.into_inner().into_inner().into_inner())),
            None => bail!("no pts found"),
        })
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
        find_main_pmt_pid(cueable_packets)
            .and_then(|(pids, s)| {
                find_meta(pids, s).and_then(|(meta, s)| {
                    let packets = s.cue_up();
                    let cueable_packets = cueable(packets);
                    find_first_picture_pts(meta.video_pid, cueable_packets).and_then(
                        move |(pts, s)| {
                            let packets = s.cue_up();
                            process_captions(meta.caption_pid, pts, packets)
                        },
                    )
                })
            })
            .map_err(|e| info!("error: {}", e))
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
