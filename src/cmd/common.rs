use anyhow::{bail, Result};
use log::{debug, info};
use tokio_stream::{Stream, StreamExt};

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

pub async fn find_main_meta<S: Stream<Item = ts::TSPacket> + Unpin>(s: &mut S) -> Result<Meta> {
    let pid = find_main_pmt_pid(s).await?;
    find_meta(pid, s).await
}

async fn find_meta<S: Stream<Item = ts::TSPacket> + Unpin>(pid: u16, s: &mut S) -> Result<Meta> {
    let pmt_stream = s.filter(move |packet| packet.pid == pid);
    let mut buffer = psi::Buffer::new(pmt_stream);
    loop {
        match buffer.next().await {
            Some(Ok(bytes)) => {
                let bytes = &bytes[..];
                let table_id = bytes[0];
                if table_id == psi::TS_PROGRAM_MAP_SECTION {
                    let pms = match psi::TSProgramMapSection::parse(bytes) {
                        Ok(pms) => pms,
                        Err(e) => {
                            info!("pmt parse error: {:?}", e);
                            continue;
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
                            return Ok(Meta {
                                audio_pid,
                                video_pid,
                                caption_pid,
                            });
                        }
                        _ => {}
                    }
                }
            }
            Some(Err(e)) => return Err(e),
            None => bail!("no meta found"),
        }
    }
}

async fn find_main_pmt_pid<S: Stream<Item = ts::TSPacket> + Unpin>(s: &mut S) -> Result<u16> {
    let pat_stream = s.filter(|packet| packet.pid == ts::PAT_PID);
    let mut buffer = psi::Buffer::new(pat_stream);
    loop {
        match buffer.next().await {
            Some(Ok(bytes)) => {
                let bytes = &bytes[..];
                let table_id = bytes[0];
                if table_id == psi::PROGRAM_ASSOCIATION_SECTION {
                    let pas = match psi::ProgramAssociationSection::parse(bytes) {
                        Ok(pas) => pas,
                        Err(e) => {
                            info!("pat parse error: {:?}", e);
                            continue;
                        }
                    };
                    for (program_number, pid) in pas.program_association {
                        if program_number != 0 {
                            // not network pid
                            debug!("main pmt: pid={}, program_number={}", pid, program_number);
                            return Ok(pid);
                        }
                    }
                }
            }
            Some(Err(e)) => return Err(e),
            None => bail!("no pid found"),
        }
    }
}

pub async fn find_first_picture_pts<S: Stream<Item = ts::TSPacket> + Unpin>(
    pid: u16,
    s: &mut S,
) -> Result<u64> {
    let video_stream = s.filter(move |packet| packet.pid == pid);
    let mut buffer = pes::Buffer::new(video_stream);
    loop {
        match buffer.next().await {
            Some(Ok(bytes)) => {
                let pes = match pes::PESPacket::parse(&bytes[..]) {
                    Ok(pes) => pes,
                    Err(e) => {
                        info!("pes parse error: {:?}", e);
                        continue;
                    }
                };
                if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
                    if h262::is_i_picture(body.pes_packet_data_byte) {
                        if let Some(pts) = pes.get_pts() {
                            return Ok(pts);
                        }
                    }
                }
            }
            Some(Err(e)) => return Err(e),
            None => bail!("no pts found"),
        }
    }
}

// FIXME: erroneous packets will be error, this function should be removed.
pub fn strip_error_packets<S: Stream<Item = Result<ts::TSPacket>>>(
    s: S,
) -> impl Stream<Item = ts::TSPacket> {
    s.filter_map(|x| if let Ok(x) = x { Some(x) } else { None })
}
