use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use anyhow::{bail, Result};
use chrono;
use chrono::offset::FixedOffset;
use chrono::DateTime;
use log::info;
use serde_derive::Serialize;
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::codec::FramedRead;

use super::common::strip_error_packets;
use super::io::path_to_async_read;
use crate::arib;
use crate::psi;
use crate::stream::cueable;
use crate::ts;
use psi::descriptor::Genre;

#[derive(Debug)]
struct Duration(chrono::Duration);

impl serde::Serialize for Duration {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0.num_seconds())
    }
}

#[derive(Debug, Serialize)]
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

fn stringify_genre(genre: &Genre) -> &'static str {
    match genre {
        Genre::News => "news",
        Genre::Sports => "sports",
        Genre::Information => "information",
        Genre::Drama => "drama",
        Genre::Music => "music",
        Genre::Variety => "variety",
        Genre::Movies => "movies",
        Genre::Animation => "animation",
        Genre::Documentary => "documentary",
        Genre::Theatre => "theatre",
        Genre::Hobby => "hobby",
        Genre::Welfare => "welfare",
        Genre::Reserved => "reserved",
        Genre::Extention => "extention",
        Genre::Others => "others",
    }
}

fn decode_to_utf8<'a, I: Iterator<Item = &'a u8>>(i: I) -> Result<String> {
    let decoder = arib::string::AribDecoder::with_event_initialization();
    decoder.decode(i)
}

fn try_into_event(eit: psi::EventInformationSection) -> Result<Vec<Event>> {
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
                            let d = decode_to_utf8(item_descs.iter().cloned().flatten())?;
                            let i = decode_to_utf8(items.iter().cloned().flatten())?;
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
                    event.title = decode_to_utf8(e.event_name.iter())?;
                    event.summary = decode_to_utf8(e.text.iter())?;
                }
                psi::Descriptor::ContentDescriptor(c) => {
                    if event.category.is_empty() && !c.items.is_empty() {
                        event.category = String::from(stringify_genre(&c.items[0]));
                    }
                }
                _ => {}
            }
        }
        let d = decode_to_utf8(item_descs.iter().cloned().flatten())?;
        let i = decode_to_utf8(items.iter().cloned().flatten())?;
        if !d.is_empty() && !i.is_empty() {
            event.detail.insert(d, i);
        }
        events.push(event)
    }
    Ok(events)
}

async fn find_service_ids<S: Stream<Item = ts::TSPacket> + Unpin>(s: &mut S) -> Result<Vec<u16>> {
    let sdt_stream = s.filter(|packet| packet.pid == psi::SDT_PID);
    let mut buffer = psi::Buffer::new(sdt_stream);
    loop {
        match buffer.next().await {
            Some(Ok(bytes)) => {
                let bytes = &bytes[..];
                let table_id = bytes[0];
                if table_id == psi::SELF_STREAM_TABLE_ID {
                    match psi::ServiceDescriptionSection::parse(bytes) {
                        Ok(sdt) => return Ok(sdt.services.iter().map(|s| s.service_id).collect()),
                        Err(e) => info!("sdt parse error: {:?}", e),
                    }
                }
            }
            Some(Err(e)) => {
                info!("find_service_id: {:?}", e);
            }
            None => bail!("no sid found"),
        }
    }
}

fn packets_to_events<S: Stream<Item = ts::TSPacket> + Unpin>(
    sids: Vec<u16>,
    s: S,
) -> impl Stream<Item = Vec<Event>> {
    psi::Buffer::new(s).filter_map(move |bytes| match bytes {
        Ok(bytes) => {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            if 0x4e <= table_id && table_id <= 0x6f {
                match psi::EventInformationSection::parse(bytes) {
                    Ok(eit) => {
                        if sids.contains(&eit.service_id) {
                            if let Ok(events) = try_into_event(eit) {
                                return Some(events);
                            }
                        }
                    }
                    Err(e) => {
                        info!("eit parse error: {:?}", e);
                    }
                }
            }
            None
        }
        Err(e) => {
            info!("packets_to_events: {:?}", e);
            None
        }
    })
}

fn into_event_stream<S: Stream<Item = ts::TSPacket> + Send + 'static + Unpin>(
    service_ids: Vec<u16>,
    mut s: S,
) -> impl Stream<Item = Vec<Event>> {
    let (event_tx, event_rx) = channel(1);
    let mut tx_map = HashMap::new();
    for pid in ts::EIT_PIDS.iter() {
        let (tx, rx) = channel(1);
        tx_map.insert(pid, tx);
        let mut events_stream = packets_to_events(service_ids.clone(), ReceiverStream::new(rx));
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(events) = events_stream.next().await {
                if event_tx.send(events).await.is_err() {
                    break;
                }
            }
        });
    }

    tokio::spawn(async move {
        while let Some(packet) = s.next().await {
            if let Some(tx) = tx_map.get_mut(&packet.pid) {
                if tx.send(packet).await.is_err() {
                    break;
                }
            }
        }
    });

    ReceiverStream::new(event_rx)
}

async fn into_event_map<S: Stream<Item = Vec<Event>> + Unpin>(
    mut s: S,
) -> Result<BTreeMap<u16, Event>> {
    let mut out = BTreeMap::new();
    while let Some(events) = s.next().await {
        for event in events.into_iter() {
            out.insert(event.id, event);
        }
    }
    Ok(out)
}

pub async fn run(input: Option<PathBuf>) -> Result<()> {
    let input = path_to_async_read(input).await?;
    let packets = FramedRead::new(input, ts::TSPacketDecoder::new());
    let packets = strip_error_packets(packets);
    let mut cueable_packets = cueable(packets);
    let sids = find_service_ids(&mut cueable_packets).await?;
    let packets = cueable_packets.cue_up();
    let events = into_event_stream(sids, packets);
    let event_map = into_event_map(events).await?;
    for e in event_map.values() {
        println!("{}", serde_json::to_string(e)?);
    }
    Ok(())
}
