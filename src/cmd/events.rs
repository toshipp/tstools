use env_logger;
use log::info;

use std::collections::BTreeMap;

use futures::future::lazy;

use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::stream::iter_ok;
use tokio::prelude::Stream;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Receiver};

use chrono;
use chrono::offset::FixedOffset;
use chrono::DateTime;

use serde_derive::Serialize;

use failure::{bail, Error, Fail};

use crate::arib;
use crate::psi;
use crate::stream::cueable;
use crate::ts;

struct Duration(chrono::Duration);

impl serde::Serialize for Duration {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0.num_seconds())
    }
}

#[derive(Serialize)]
struct Event {
    id: u16,
    start: DateTime<FixedOffset>,
    duration: Duration,
    title: String,
    summary: String,
    detail: BTreeMap<String, String>,
    category: String,
}

impl Event {
    fn new(id: u16, start: DateTime<FixedOffset>, duration: chrono::Duration) -> Self {
        Event {
            id,
            start,
            duration: Duration(duration),
            title: String::new(),
            summary: String::new(),
            detail: BTreeMap::new(),
            category: String::new(),
        }
    }
}

fn try_into_event(eit: psi::EventInformationSection) -> Result<Vec<Event>, Error> {
    let mut events = Vec::new();
    for eit_event in eit.events {
        if eit_event.start_time.is_none() || eit_event.duration.is_none() {
            continue;
        }
        let mut event = Event::new(
            eit_event.event_id,
            eit_event.start_time.unwrap(),
            eit_event.duration.unwrap(),
        );
        let mut item_descs = Vec::new();
        let mut items = Vec::new();
        for desc in eit_event.descriptors.iter() {
            match desc {
                psi::Descriptor::ExtendedEventDescriptor(e) => {
                    for item in e.items.iter() {
                        if !item.item_description.is_empty() {
                            let d =
                                arib::string::decode_to_utf8(item_descs.iter().cloned().flatten())?;
                            let i = arib::string::decode_to_utf8(items.iter().cloned().flatten())?;
                            if !d.is_empty() && !i.is_empty() {
                                event.detail.insert(d, i);
                            }
                            item_descs.clear();
                            items.clear();
                        }
                        item_descs.push(item.item_description);
                        items.push(item.item);
                    }
                }
                psi::Descriptor::ShortEventDescriptor(e) => {
                    event.title = format!("{:?}", e.event_name);
                    event.summary = format!("{:?}", e.text);
                }
                psi::Descriptor::ContentDescriptor(c) => {
                    if event.category.is_empty() && !c.items.is_empty() {
                        event.category = format!("{:?}", c.items[0]);
                    }
                }
                _ => {}
            }
        }
        let d = arib::string::decode_to_utf8(item_descs.iter().cloned().flatten())?;
        let i = arib::string::decode_to_utf8(items.iter().cloned().flatten())?;
        if !d.is_empty() && !i.is_empty() {
            event.detail.insert(d, i);
        }
        events.push(event)
    }
    Ok(events)
}

fn find_service_id<S: Stream<Item = ts::TSPacket, Error = Error>>(
    s: S,
) -> impl Future<Item = (u16, S), Error = Error> {
    let sdt_stream = s.filter(|packet| packet.pid == psi::SDT_PID);
    psi::Buffer::new(sdt_stream)
        .filter_map(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            if table_id == psi::SELF_STREAM_TABLE_ID {
                match psi::ServiceDescriptionSection::parse(bytes) {
                    Ok(sdt) => return Some(sdt.services[0].service_id),
                    Err(e) => info!("sdt parse error: {:?}", e),
                }
            }
            None
        })
        .into_future()
        .map(|(sid, stream)| (sid, stream.into_inner().into_inner().into_inner()))
        .map_err(|(e, _)| e)
        .and_then(|(sid, s)| match sid {
            Some(sid) => Ok((sid, s)),
            None => bail!("no sid found"),
        })
}

fn packets_to_events<S: Stream<Item = ts::TSPacket, Error = E>, E: Fail>(
    sid: u16,
    s: S,
) -> impl Stream<Item = Event, Error = Error> {
    psi::Buffer::new(s)
        .filter_map(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            if 0x4e <= table_id && table_id <= 0x6f {
                match psi::EventInformationSection::parse(bytes) {
                    Ok(eit) => {
                        if eit.service_id == sid {
                            match try_into_event(eit) {
                                Ok(events) => return Some(events),
                                Err(e) => info!("can not convert events: {:?}", e),
                            }
                        }
                    }
                    Err(e) => info!("eit parse error: {:?}", e),
                }
            }
            None
        })
        .map(iter_ok)
        .flatten()
}

fn into_event_stream<S: Stream<Item = ts::TSPacket, Error = Error> + Send + 'static>(
    s: S,
    service_id: u16,
) -> Receiver<Event> {
    let (event_tx, event_rx) = channel(1);
    let demuxer = ts::demuxer::Demuxer::new(move |pid: u16| -> Result<_, Error> {
        if ts::EIT_PIDS.iter().any(|x| *x == pid) {
            let (tx, rx) = channel(1);
            tokio::spawn(
                packets_to_events(service_id, rx)
                    .map_err(|e| Error::from(e))
                    .forward(event_tx.clone())
                    .map(|_| ())
                    .map_err(|e| info!("can not convert packets into stream: {:?}", e)),
            );
            return Ok(Some(tx));
        }
        Ok(None)
    });
    tokio::spawn(
        s.map_err(|e| Error::from(e))
            .forward(demuxer)
            .map(|_| ())
            .map_err(|e| info!("can not demux: {:?}", e)),
    );
    event_rx
}

fn into_event_map<S: Stream<Item = Event, Error = E>, E: Fail>(
    s: S,
) -> impl Future<Item = BTreeMap<u16, Event>, Error = E> {
    s.fold(BTreeMap::new(), |mut out, event| {
        out.insert(event.id, event);
        Ok(out)
    })
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let packets = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        let cueable_packets = cueable(packets);
        find_service_id(cueable_packets)
            .and_then(|(sid, s)| {
                let packets = s.cue_up();
                let events = into_event_stream(packets, sid);
                into_event_map(events).map_err(|e| Error::from(e))
            })
            .map(|event_map| {
                for e in event_map.values() {
                    println!("{}", serde_json::to_string(e).unwrap());
                }
            })
            .map_err(|e| info!("error: {:?}", e))
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
