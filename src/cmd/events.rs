use env_logger;
use log::info;

use std::sync::Arc;
use std::sync::Mutex;

use std::collections::BTreeMap;

use futures::future::lazy;

use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Sender};

use chrono;
use chrono::offset::FixedOffset;
use chrono::DateTime;

use serde_derive::Serialize;

use crate::arib;
use crate::psi;
use crate::ts;

struct Duration(chrono::Duration);

impl serde::Serialize for Duration {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0.num_seconds())
    }
}

#[derive(Serialize)]
struct Event {
    start: DateTime<FixedOffset>,
    duration: Duration,
    title: String,
    summary: String,
    detail: BTreeMap<String, String>,
    category: String,
}

impl Event {
    fn new(start: DateTime<FixedOffset>, duration: chrono::Duration) -> Self {
        Event {
            start,
            duration: Duration(duration),
            title: String::new(),
            summary: String::new(),
            detail: BTreeMap::new(),
            category: String::new(),
        }
    }
}

fn process_eit(
    events: &mut BTreeMap<u16, Event>,
    eit: psi::EventInformationSection,
) -> Result<(), failure::Error> {
    for event in eit.events {
        if event.start_time.is_none() || event.duration.is_none() {
            continue;
        }
        let mut record = events.entry(event.event_id).or_insert(Event::new(
            event.start_time.unwrap(),
            event.duration.unwrap(),
        ));

        let mut item_descs = Vec::new();
        let mut items = Vec::new();
        for desc in event.descriptors.iter() {
            match desc {
                psi::Descriptor::ExtendedEventDescriptor(e) => {
                    for item in e.items.iter() {
                        if !item.item_description.is_empty() {
                            let d =
                                arib::string::decode_to_utf8(item_descs.iter().cloned().flatten())?;
                            let i = arib::string::decode_to_utf8(items.iter().cloned().flatten())?;
                            if !d.is_empty() && !i.is_empty() {
                                record.detail.insert(d, i);
                            }
                            item_descs.clear();
                            items.clear();
                        }
                        item_descs.push(item.item_description);
                        items.push(item.item);
                    }
                }
                psi::Descriptor::ShortEventDescriptor(e) => {
                    record.title = format!("{:?}", e.event_name);
                    record.summary = format!("{:?}", e.text);
                }
                psi::Descriptor::ContentDescriptor(c) => {
                    if record.category.is_empty() && !c.items.is_empty() {
                        record.category = format!("{:?}", c.items[0]);
                    }
                }
                _ => {}
            }
        }
        let d = arib::string::decode_to_utf8(item_descs.iter().cloned().flatten())?;
        let i = arib::string::decode_to_utf8(items.iter().cloned().flatten())?;
        if !d.is_empty() && !i.is_empty() {
            record.detail.insert(d, i);
        }
    }
    Ok(())
}

struct SinkMaker {
    service_id: Arc<Mutex<Option<u16>>>,
    events: Arc<Mutex<BTreeMap<u16, Event>>>,
}

impl SinkMaker {
    fn new() -> Self {
        Self {
            service_id: Arc::new(Mutex::new(None)),
            events: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn events(&self) -> Arc<Mutex<BTreeMap<u16, Event>>> {
        self.events.clone()
    }

    fn make_eit_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let service_id = self.service_id.clone();
        let events = self.events.clone();
        tokio::spawn(
            psi::Buffer::new(rx)
                .for_each(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    if 0x4e <= table_id && table_id <= 0x6f {
                        let eit = psi::EventInformationSection::parse(bytes)?;
                        let service_id = service_id.lock().unwrap();
                        if let Some(id) = *service_id {
                            if id == eit.service_id {
                                let mut events = events.lock().unwrap();
                                process_eit(&mut events, eit)?;
                            }
                        }
                    }
                    Ok(())
                })
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_sdt_sink(&self) -> Sender<ts::TSPacket> {
        let (tx, rx) = channel(1);
        let service_id = self.service_id.clone();
        tokio::spawn(
            psi::Buffer::new(rx)
                .for_each(move |bytes| {
                    let bytes = &bytes[..];
                    let table_id = bytes[0];
                    if table_id == psi::SELF_STREAM_TABLE_ID {
                        let sdt = psi::ServiceDescriptionSection::parse(bytes)?;
                        let mut service_id = service_id.lock().unwrap();
                        if service_id.is_none() && !sdt.services.is_empty() {
                            *service_id = Some(sdt.services[0].service_id);
                        }
                    }
                    Ok(())
                })
                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
        );
        tx
    }

    fn make_sink(&self, pid: u16) -> Option<Sender<ts::TSPacket>> {
        if ts::EIT_PIDS.iter().any(|x| *x == pid) {
            return Some(self.make_eit_sink());
        }
        if pid == psi::SDT_PID {
            return Some(self.make_sdt_sink());
        }
        None
    }
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let sink_maker = SinkMaker::new();
        let events = sink_maker.events();
        let demuxer = ts::demuxer::Demuxer::new(move |pid: u16| Ok(sink_maker.make_sink(pid)));

        let decoder = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        decoder.forward(demuxer).then(move |ret| {
            if let Err(e) = ret {
                info!("error: {}", e);
            }
            let events = events.lock().unwrap();
            for e in events.values() {
                println!("{}", serde_json::to_string(e).unwrap());
            }
            Ok(())
        })
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
