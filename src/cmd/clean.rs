use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{bail, Result};
use bytes::{Bytes, BytesMut};
use log::info;
use tokio;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::codec::FramedRead;

use super::common::strip_error_packets;
use super::io::{path_to_async_read, path_to_async_write};
use crate::crc32;
use crate::psi;
use crate::stream::cueable;
use crate::ts;

async fn find_pids_from_pat<S: Stream<Item = ts::TSPacket> + Unpin>(
    s: &mut S,
    service_index: Option<usize>,
) -> Result<(Option<u16>, HashSet<u16>)> {
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
                    let mut network_pid = None;
                    let mut pmt_pids = HashSet::new();
                    let mut idx = 0usize;
                    for (program_number, pid) in pas.program_association {
                        if program_number == 0 {
                            network_pid = Some(pid);
                        } else {
                            info!(
                                "found PMT program_number={:?}, pid={:?}",
                                program_number, pid
                            );
                            if service_index.is_none() || idx == service_index.unwrap() {
                                pmt_pids.insert(pid);
                            }
                            idx += 1;
                        }
                    }

                    return Ok((network_pid, pmt_pids));
                }
            }
            Some(Err(e)) => return Err(e.into()),
            None => bail!("no pids found"),
        }
    }
}

async fn find_keep_pids_from_pmt<S: Stream<Item = ts::TSPacket> + Unpin>(
    pmt_pid: u16,
    pmt_stream: S,
) -> Result<HashSet<u16>> {
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
                    let mut pids = HashSet::new();
                    pids.insert(pmt_pid);
                    pids.insert(pms.pcr_pid);
                    for si in pms.stream_info.iter() {
                        if si.stream_type == psi::STREAM_TYPE_H264 {
                            // if the video stream is h264, ignore this program.
                            return Ok(HashSet::new());
                        }
                        pids.insert(si.elementary_pid);
                    }
                    return Ok(pids);
                }
            }
            Some(Err(e)) => return Err(e.into()),
            None => bail!("no keep pids found"),
        }
    }
}

async fn find_keep_pids_from_pmts<S: Stream<Item = ts::TSPacket> + Unpin>(
    pmt_pids: HashSet<u16>,
    s: &mut S,
) -> Result<HashSet<u16>> {
    let mut handles = Vec::new();
    let mut tx_map = HashMap::new();
    for pid in pmt_pids.iter() {
        let (tx, rx) = channel(1);
        tx_map.insert(pid, tx);
        handles.push(tokio::spawn(find_keep_pids_from_pmt(
            *pid,
            ReceiverStream::new(rx),
        )));
    }

    let transfer = async move {
        while !tx_map.is_empty() {
            if let Some(packet) = s.next().await {
                let pid = packet.pid;
                if let Some(tx) = tx_map.get_mut(&pid) {
                    if tx.send(packet).await.is_err() {
                        tx_map.remove(&pid);
                    }
                }
            }
        }
    };

    let receiver = async move {
        let mut pids = HashSet::new();
        for handle in handles.into_iter() {
            for pid in handle.await??.into_iter() {
                pids.insert(pid);
            }
        }
        Ok(pids)
    };

    tokio::join!(transfer, receiver).1
}

async fn find_keep_pids<S: Stream<Item = ts::TSPacket> + Unpin>(
    s: &mut S,
    service_index: Option<usize>,
) -> Result<HashSet<u16>> {
    let (network_pid, pmt_pids) = find_pids_from_pat(s, service_index).await?;
    let mut keep_pids = find_keep_pids_from_pmts(pmt_pids, s).await?;
    if let Some(network_pid) = network_pid {
        keep_pids.insert(network_pid);
    }
    Ok(keep_pids)
}

fn retain_keep_pids(packet: ts::TSPacket, pids: &HashSet<u16>) -> Bytes {
    let mut out = BytesMut::with_capacity(ts::TS_PACKET_LENGTH);

    let bytes = packet.into_raw();
    let adaptation_field_control = (bytes[3] & 0x30) >> 4;
    let data_offset = match adaptation_field_control {
        0b10 | 0b11 => 4 + 1 + usize::from(bytes[4]),
        _ => 4,
    };
    let data = &bytes[data_offset..];
    let pat_offset = data_offset + 1 + usize::from(data[0]);
    let pat = &bytes[pat_offset..];
    let section_length = (usize::from(pat[1] & 0xf) << 8) | usize::from(pat[2]);

    // copy data before the map.
    out.extend_from_slice(&bytes[..pat_offset + 8]);

    let mut map = &pat[8..3 + section_length - 4];
    let mut new_map_bytes: usize = 0;
    while map.len() > 0 {
        let program_number = (u16::from(map[0]) << 8) | u16::from(map[1]);
        let pid = (u16::from(map[2] & 0x1f) << 8) | u16::from(map[3]);
        if program_number == 0 || pids.contains(&pid) {
            out.extend_from_slice(&map[0..4]);
            new_map_bytes += 4;
        }
        map = &map[4..];
    }

    // set new section_length
    let new_section_length = 5 + new_map_bytes + 4;
    out[pat_offset + 1] &= 0xf0;
    out[pat_offset + 1] |= (new_section_length >> 8) as u8;
    out[pat_offset + 2] = new_section_length as u8;

    let crc = crc32::crc32(&out[pat_offset..pat_offset + 3 + new_section_length - 4]);
    out.extend_from_slice(&crc.to_be_bytes()[..]);

    // fill padding.
    out.resize(ts::TS_PACKET_LENGTH, 0);

    out.freeze()
}

async fn dump_packets<S: Stream<Item = ts::TSPacket> + Unpin>(
    mut s: S,
    pids: HashSet<u16>,
    mut out: File,
) -> Result<()> {
    while let Some(packet) = s.next().await {
        if packet.pid == ts::PAT_PID {
            if !packet.transport_error_indicator {
                out.write(&retain_keep_pids(packet, &pids)[..]).await?;
            }
        } else if pids.contains(&packet.pid) {
            out.write(&packet.into_raw()[..]).await?;
        }
    }
    Ok(())
}

pub async fn run(
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    service_index: Option<usize>,
) -> Result<()> {
    let input = path_to_async_read(input).await?;
    let output = path_to_async_write(output).await?;
    let packets = FramedRead::new(input, ts::TSPacketDecoder::new());
    let packets = strip_error_packets(packets);
    let mut cueable_packets = cueable(packets);
    let pids = find_keep_pids(&mut cueable_packets, service_index).await?;
    let packets = cueable_packets.cue_up();
    dump_packets(packets, pids, output).await
}
