use env_logger;
use log::info;

use futures::Future;
use tokio::codec::{BytesCodec, FramedRead, FramedWrite};
use tokio::io::{stdin, stdout};
use tokio::prelude::{AsyncWrite, Sink, Stream};
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Sender};

use bytes::{Bytes, BytesMut};

use std::collections::HashSet;
use std::mem;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use failure::{bail, Error};

use crate::crc32;
use crate::psi;
use crate::stream::{cueable, interruptible, Cued};
use crate::ts;

struct FindKeepPidsMaker {
    pmt_pids: Arc<Mutex<HashSet<u16>>>,
    remaining_pids: Arc<AtomicU16>,
    pids: Arc<Mutex<HashSet<u16>>>,
    pids_tx: Sender<HashSet<u16>>,
}

impl FindKeepPidsMaker {
    fn new(tx: Sender<HashSet<u16>>) -> FindKeepPidsMaker {
        FindKeepPidsMaker {
            pmt_pids: Arc::new(Mutex::new(HashSet::new())),
            remaining_pids: Arc::new(AtomicU16::new(0)),
            pids: Arc::new(Mutex::new(HashSet::new())),
            pids_tx: tx,
        }
    }

    fn make_pat_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let pmt_pids = self.pmt_pids.clone();
        let pids = self.pids.clone();
        let remaining_pids = self.remaining_pids.clone();
        tokio::spawn(
            psi::Buffer::new(rx)
                .take_while(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    if table_id == psi::PROGRAM_ASSOCIATION_SECTION {
                        match psi::ProgramAssociationSection::parse(bytes) {
                            Ok(pas) => {
                                for (program_number, pid) in pas.program_association {
                                    let mut pids = pids.lock().unwrap();
                                    if program_number == 0 {
                                        pids.insert(pid);
                                    } else {
                                        let mut pmt_pids = pmt_pids.lock().unwrap();
                                        pmt_pids.insert(dbg!(pid));
                                        remaining_pids.fetch_add(1, Ordering::Release);
                                    }
                                }
                                return Ok(dbg!(false));
                            }
                            Err(e) => {
                                info!("err {}: {:#?}", line!(), e);
                            }
                        };
                    }
                    Ok(true)
                })
                .for_each(|_| Ok(()))
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_pmt_sink(&self, pid: u16) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let pids2 = self.pids.clone();
        let pids_tx = self.pids_tx.clone();
        let remaining_pids1 = self.remaining_pids.clone();
        let remaining_pids2 = self.remaining_pids.clone();
        let mut found = false;
        tokio::spawn(
            psi::Buffer::new(rx)
                .map(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    let mut pids = Vec::new();
                    if !found && table_id == psi::TS_PROGRAM_MAP_SECTION {
                        match psi::TSProgramMapSection::parse(bytes) {
                            Ok(pms) => {
                                found = true;
                                pids.push(pid);
                                pids.push(pms.pcr_pid);
                                for si in pms.stream_info.iter() {
                                    if si.stream_type == psi::STREAM_TYPE_H264 {
                                        remaining_pids1.fetch_sub(1, Ordering::AcqRel);
                                        return Vec::new();
                                    }
                                    pids.push(si.elementary_pid);
                                }
                                remaining_pids1.fetch_sub(1, Ordering::AcqRel);
                            }
                            Err(e) => {
                                info!("err {}: {:#?}", line!(), e);
                            }
                        }
                    }
                    pids
                })
                .for_each(move |pids| {
                    let mut keep_pids = pids2.lock().unwrap();
                    for pid in pids.into_iter() {
                        keep_pids.insert(pid);
                    }

                    let pids = match dbg!(remaining_pids2.load(Ordering::Acquire)) {
                        0 => {
                            let pids = dbg!(mem::replace(&mut *keep_pids, HashSet::new()));
                            pids
                        }
                        _ => HashSet::new(),
                    };
                    pids_tx
                        .clone()
                        .send(pids)
                        .map(|_| ())
                        .map_err(|e| Error::from(e))
                })
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_sink(&mut self, pid: u16) -> Result<Option<Sender<ts::TSPacket>>, Error> {
        if pid == 0 {
            return Ok(Some(self.make_pat_sink()));
        }
        {
            let pmt_pids = self.pmt_pids.lock().unwrap();
            if pmt_pids.contains(&pid) {
                return Ok(Some(self.make_pmt_sink(pid)));
            }
        }
        Ok(None)
    }
}

fn find_keep_pids<S: Stream<Item = ts::TSPacket, Error = Error>>(
    s: S,
) -> impl Future<Item = (Cued<S>, HashSet<u16>), Error = Error> {
    let (s, interuppter) = interruptible(cueable(s));
    let (tx, rx) = channel(1);
    let mut sink_maker = FindKeepPidsMaker::new(tx);
    let demuxer = ts::demuxer::Demuxer::new(move |pid: u16| sink_maker.make_sink(pid));
    let pids_future = rx
        .filter(|pids| !pids.is_empty())
        .into_future()
        .map(move |(x, _)| {
            interuppter.interrupt();
            dbg!(x)
        })
        .map_err(|(e, _)| info!("err: {:?}", e));
    s.forward(demuxer)
        .map(|(s, _)| s.into_inner().cue_up())
        .map_err(|e| info!("err: {:?}", e))
        .join(pids_future)
        .then(|r| match r {
            Ok((s, Some(pids))) => Ok((s, dbg!(pids))),
            _ => bail!("avpid not found"),
        })
}

fn dump_pat(packet: ts::TSPacket, pids: &HashSet<u16>) -> Bytes {
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

fn dump_packets<S: Stream<Item = ts::TSPacket, Error = Error>, W: AsyncWrite>(
    s: S,
    pids: HashSet<u16>,
    out: W,
) -> impl Future<Item = (), Error = Error> {
    s.filter_map(move |packet| {
        (if packet.pid == ts::PAT_PID {
            Some(dump_pat(packet, &pids))
        } else if pids.contains(&packet.pid) {
            Some(packet.into_raw())
        } else {
            None
        })
    })
    .forward(FramedWrite::new(out, BytesCodec::new()))
    .map(|_| ())
    .map_err(|e| Error::from(e))
}

pub fn run() {
    env_logger::init();

    let proc = find_keep_pids(FramedRead::new(stdin(), ts::TSPacketDecoder::new()))
        .map_err(|e| info!("err: {:?}", e))
        .and_then(|(s, pids)| dump_packets(s, pids, stdout()).map_err(|e| info!("{:?}", e)));

    let mut rt = Builder::new().core_threads(2).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
