use failure::{bail, Error};
use log::{debug, info};
use tokio::prelude::{Future, Stream};

use crate::arib::caption::is_caption;
use crate::h262;
use crate::pes;
use crate::psi;
use crate::ts;

pub struct Meta {
    pub audio_pid: u16,
    pub video_pid: u16,
    pub caption_pid: u16,
}

pub fn find_main_meta<S: Stream<Item = ts::TSPacket, Error = Error>>(
    s: S,
) -> impl Future<Item = (Meta, S), Error = Error> {
    find_main_pmt_pid(s).and_then(|(pids, s)| find_meta(pids, s))
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
                debug!("stream info: {:#?}", pms.stream_info);
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
                        });
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
                        debug!("main pmt: pid={}, program_number={}", pid, program_number);
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

pub fn find_first_picture_pts<S: Stream<Item = ts::TSPacket, Error = Error>>(
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

// FIXME: erroneous packets will be error, this function should be removed.
pub fn strip_error_packets<S: Stream<Item = ts::TSPacket, Error = Error>>(
    s: S,
) -> impl Stream<Item = S::Item, Error = S::Error> {
    s.filter(|x| !x.transport_error_indicator)
}
