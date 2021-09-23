use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{bail, Error};
use bytes::{Bytes, BytesMut};
use futures::future::lazy;
use futures::Future;
use log::info;
use tokio::codec::{BytesCodec, FramedRead, FramedWrite};
use tokio::prelude::{Async, AsyncSink, AsyncWrite, Sink, Stream};
use tokio::runtime::Builder;
use tokio::sync::mpsc::channel;

use super::common::strip_error_packets;
use super::io::{path_to_async_read, path_to_async_write};
use crate::crc32;
use crate::psi;
use crate::stream::cueable;
use crate::ts;

struct ForwardUntilResolved<T: Stream, U, F> {
    stream: Option<T>,
    sink: Option<U>,
    future: F,
    forward_item: Option<T::Item>,
    forward_done: bool,
}

impl<T: Stream, U, F> ForwardUntilResolved<T, U, F> {
    fn new(stream: T, sink: U, future: F) -> Self {
        Self {
            stream: Some(stream),
            sink: Some(sink),
            future,
            forward_item: None,
            forward_done: false,
        }
    }

    fn stream_mut(&mut self) -> &mut T {
        self.stream.as_mut().take().unwrap()
    }

    fn sink_mut(&mut self) -> &mut U {
        self.sink.as_mut().take().unwrap()
    }
}

impl<T, U, F> Future for ForwardUntilResolved<T, U, F>
where
    T: Stream,
    U: Sink<SinkItem = T::Item>,
    F: Future,
    F::Error: From<T::Error>,
    F::Error: From<U::SinkError>,
{
    type Item = (F::Item, T, U);
    type Error = F::Error;

    fn poll(&mut self) -> Result<Async<Self::Item>, Self::Error> {
        loop {
            match self.future.poll()? {
                Async::Ready(item) => {
                    let stream = self.stream.take().unwrap();
                    let sink = self.sink.take().unwrap();
                    return Ok(Async::Ready((item, stream, sink)));
                }
                Async::NotReady => {
                    if self.forward_done {
                        return Ok(Async::NotReady);
                    }
                }
            }

            match self.forward_item.take() {
                Some(item) => match self.sink_mut().start_send(item)? {
                    AsyncSink::Ready => continue,
                    AsyncSink::NotReady(item) => {
                        self.forward_item = Some(item);
                        return Ok(Async::NotReady);
                    }
                },
                None => {}
            }

            match self.stream_mut().poll()? {
                Async::Ready(Some(item)) => match self.sink_mut().start_send(item)? {
                    AsyncSink::Ready => continue,
                    AsyncSink::NotReady(item) => {
                        self.forward_item = Some(item);
                        return Ok(Async::NotReady);
                    }
                },
                Async::Ready(None) => match self.sink_mut().close()? {
                    Async::Ready(_) => {
                        self.forward_done = true;
                        continue;
                    }
                    Async::NotReady => {
                        return Ok(Async::NotReady);
                    }
                },
                Async::NotReady => match self.sink_mut().poll_complete()? {
                    Async::Ready(_) => continue,
                    Async::NotReady => return Ok(Async::NotReady),
                },
            }
        }
    }
}

fn find_pids_from_pat<S: Stream<Item = ts::TSPacket, Error = Error>>(
    s: S,
) -> impl Future<Item = (Option<u16>, HashSet<u16>, S), Error = Error> {
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
                let mut network_pid = None;
                let mut pmt_pids = HashSet::new();
                for (program_number, pid) in pas.program_association {
                    if program_number == 0 {
                        network_pid = Some(pid);
                    } else {
                        pmt_pids.insert(pid);
                    }
                }

                return Some((network_pid, pmt_pids));
            }
            None
        })
        .into_future()
        .map_err(|(e, _)| e)
        .and_then(|(pids, s)| match pids {
            Some((network_pid, pmt_pids)) => Ok((
                network_pid,
                pmt_pids,
                s.into_inner().into_inner().into_inner(),
            )),
            None => bail!("no pids found"),
        })
}

fn find_keep_pids_from_pmt<S: Stream<Item = ts::TSPacket, Error = Error>>(
    pmt_pid: u16,
    pmt_stream: S,
) -> impl Future<Item = (HashSet<u16>, S), Error = Error> {
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
                let mut pids = HashSet::new();
                pids.insert(pmt_pid);
                pids.insert(pms.pcr_pid);
                for si in pms.stream_info.iter() {
                    if si.stream_type == psi::STREAM_TYPE_H264 {
                        // if the video stream is h264, ignore this program.
                        return Some(HashSet::new());
                    }
                    pids.insert(si.elementary_pid);
                }
                return Some(pids);
            }
            None
        })
        .into_future()
        .map(|(pids, stream)| (pids, stream.into_inner().into_inner()))
        .map_err(|(e, _)| e)
        .and_then(|(pids, s)| match pids {
            Some(pids) => Ok((pids, s)),
            None => bail!("no pids found"),
        })
}

fn find_keep_pids_from_pmts<S: Stream<Item = ts::TSPacket, Error = Error>>(
    pmt_pids: HashSet<u16>,
    s: S,
) -> impl Future<Item = (HashSet<u16>, S), Error = Error> {
    let (tx, rx) = channel(1);
    let mut tx_map: HashMap<u16, _> = pmt_pids
        .into_iter()
        .map(move |pid| (pid, tx.clone()))
        .collect();
    let demuxer = ts::demuxer::Demuxer::new(move |pid: u16| match tx_map.remove_entry(&pid) {
        Some((pid, pids_tx)) => {
            let (tx, rx) = channel(1);
            tokio::spawn(
                find_keep_pids_from_pmt(pid, rx.map_err(|e| Error::from(e)))
                    .and_then(move |(pids, _)| pids_tx.send(pids).map_err(|e| Error::from(e)))
                    .map(|_| ())
                    .map_err(|e| info!("pids send error: {:?}", e)),
            );
            return Ok(Some(tx.sink_map_err(|e| Error::from(e))));
        }
        None => Ok(None),
    });
    let collect_pids = rx
        .fold(HashSet::new(), |mut out, pids| {
            for pid in pids {
                out.insert(pid);
            }
            Ok(out)
        })
        .map_err(|e| Error::from(e));
    ForwardUntilResolved::new(s, demuxer, collect_pids).map(|(pids, s, _)| (pids, s))
}

fn find_keep_pids<S: Stream<Item = ts::TSPacket, Error = Error>>(
    s: S,
) -> impl Future<Item = (HashSet<u16>, S), Error = Error> {
    find_pids_from_pat(s).and_then(|(network_pid, pmt_pids, s)| {
        find_keep_pids_from_pmts(pmt_pids, s).and_then(move |(mut pids, s)| {
            if let Some(network_pid) = network_pid {
                pids.insert(network_pid);
            }
            Ok((pids, s))
        })
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
        if packet.pid == ts::PAT_PID {
            Some(dump_pat(packet, &pids))
        } else if pids.contains(&packet.pid) {
            Some(packet.into_raw())
        } else {
            None
        }
    })
    .forward(FramedWrite::new(out, BytesCodec::new()))
    .map(|_| ())
    .map_err(|e| Error::from(e))
}

pub fn run(input: Option<PathBuf>, output: Option<PathBuf>) -> Result<(), Error> {
    let proc = lazy(|| {
        path_to_async_read(input).and_then(|input| {
            path_to_async_write(output).and_then(|output| {
                let packets = FramedRead::new(input, ts::TSPacketDecoder::new());
                let packets = strip_error_packets(packets);
                let cueable_packets = cueable(packets);
                find_keep_pids(cueable_packets).and_then(|(pids, s)| {
                    let s = s.cue_up();
                    dump_packets(s, pids, output)
                })
            })
        })
    });

    let rt = Builder::new().build()?;
    rt.block_on_all(proc)
}
