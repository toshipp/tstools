use env_logger;
use log::info;

use std::sync::Arc;
use std::sync::Mutex;

use std::fmt::Debug;

use std::collections::BTreeMap;
use std::collections::HashSet;

use futures::future::lazy;

use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;

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

struct Context {
    stream_types: HashSet<u8>,
    descriptors: HashSet<u8>,
    service_id: Option<u16>,
    events: BTreeMap<u16, Event>,
}

impl Context {
    fn new() -> Context {
        Context {
            stream_types: HashSet::new(),
            descriptors: HashSet::new(),
            service_id: None,
            events: BTreeMap::new(),
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
        let d = arib::string::decode_to_utf8(item_descs.iter().cloned().flatten()).unwrap();
        let i = arib::string::decode_to_utf8(items.iter().cloned().flatten()).unwrap();
        if !d.is_empty() && !i.is_empty() {
            record.detail.insert(d, i);
        }
    }
    Ok(())
}

fn eit_processor<S, E>(pctx: Arc<Mutex<Context>>, s: S) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s)
        .for_each(move |bytes| {
            let mut ctx = pctx.lock().unwrap();
            let bytes = &bytes[..];
            let table_id = bytes[0];
            match table_id {
                n if 0x4e <= n && n <= 0x6f => {
                    let eit = psi::EventInformationSection::parse(bytes)?;
                    if let Some(id) = ctx.service_id {
                        if id == eit.service_id {
                            return process_eit(&mut ctx.events, eit);
                        }
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

fn sdt_processor<S, E>(pctx: Arc<Mutex<Context>>, s: S) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s)
        .for_each(move |bytes| {
            let mut ctx = pctx.lock().unwrap();
            let bytes = &bytes[..];
            let table_id = bytes[0];
            match table_id {
                n if psi::SELF_STREAM_TABLE_ID == n => {
                    let sdt = psi::ServiceDescriptionSection::parse(bytes)?;
                    if ctx.service_id.is_none() && !sdt.services.is_empty() {
                        ctx.service_id = Some(sdt.services[0].service_id);
                    }
                }
                _ => {
                    unreachable!("bug");
                }
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

pub fn run() {
    env_logger::init();

    let proc = lazy(|| {
        let pctx = Arc::new(Mutex::new(Context::new()));
        let demuxer = ts::demuxer::Demuxer::new();
        let mut demux_register = demuxer.register();
        // eit
        for pid in ts::EIT_PIDS.iter() {
            tokio::spawn(eit_processor(
                pctx.clone(),
                demux_register.try_register(*pid).unwrap(),
            ));
        }

        // sdt
        tokio::spawn(sdt_processor(
            pctx.clone(),
            demux_register.try_register(psi::SDT_PID).unwrap(),
        ));

        let decoder = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        decoder.forward(demuxer).then(move |ret| {
            if let Err(e) = ret {
                info!("err: {}", e);
            }
            let ctx = pctx.lock().unwrap();
            info!("types: {:#?}", ctx.stream_types);
            info!("descriptors: {:#?}", ctx.descriptors);
            for e in ctx.events.values() {
                println!("{}", serde_json::to_string(e).unwrap());
            }
            Ok(())
        })
    });

    let mut rt = Builder::new().core_threads(1).build().unwrap();
    rt.spawn(proc);
    rt.shutdown_on_idle().wait().unwrap();
}
