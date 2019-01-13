use env_logger;
use log::{debug, info};

use serde_derive::Serialize;

use failure;
use std::cell::RefCell;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;
use tokio;
use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::{ok, Future};
use tokio::prelude::Stream;
use tokio_channel::mpsc::{channel, Sender};

use chrono::offset::FixedOffset;
use chrono::{DateTime, Duration};

use futures::future::lazy;
use futures::sink::Sink;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

#[macro_use]
mod util;
mod arib;
mod crc32;
mod pes;
mod psi;
mod ts;

const PAT_PID: u16 = 0;
#[allow(dead_code)]
const CAT_PID: u16 = 1;
#[allow(dead_code)]
const TSDT_PID: u16 = 2;

const EIT_PIDS: [u16; 3] = [0x0012, 0x0026, 0x0027];

#[allow(dead_code)]
const BS_SYS: usize = 1536;
#[allow(dead_code)]
const TB_SIZE: usize = 512;

#[allow(dead_code)]
const STREAM_TYPE_VIDEO: u8 = 0x2;
#[allow(dead_code)]
const STREAM_TYPE_PES_PRIVATE_DATA: u8 = 0x6;
#[allow(dead_code)]
const STREAM_TYPE_TYPE_D: u8 = 0xd;
#[allow(dead_code)]
const STREAM_TYPE_ADTS: u8 = 0xf;
#[allow(dead_code)]
const STREAM_TYPE_RESERVED_BEGIN: u8 = 0x15;
#[allow(dead_code)]
const STREAM_TYPE_RESERVED_END: u8 = 0x7f;
#[allow(dead_code)]
const STREAM_TYPE_H264: u8 = 0x1b;

impl serde::Serialize for SeDuration {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0.num_seconds())
    }
}

struct SeDuration(Duration);

#[derive(Serialize)]
struct Event {
    start: DateTime<FixedOffset>,
    duration: SeDuration,
    title: String,
    summary: String,
    detail: BTreeMap<String, String>,
    category: String,
}

impl Event {
    fn new(start: DateTime<FixedOffset>, duration: Duration) -> Self {
        Event {
            start,
            duration: SeDuration(duration),
            title: String::new(),
            summary: String::new(),
            detail: BTreeMap::new(),
            category: String::new(),
        }
    }
}

struct Context {
    psi_processors: HashMap<u16, Sender<ts::TSPacket>>,
    pes_processors: HashMap<u16, Sender<ts::TSPacket>>,
    stream_types: HashSet<u8>,
    descriptors: HashSet<u8>,
    pids: HashSet<u16>,
    service_id: Option<u16>,
    events: BTreeMap<u16, Event>,
}

impl Context {
    fn new() -> Context {
        Context {
            psi_processors: HashMap::new(),
            pes_processors: HashMap::new(),
            stream_types: HashSet::new(),
            descriptors: HashSet::new(),
            pids: HashSet::new(),
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
                psi::Descriptor::ExtendedEvent(e) => {
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
                psi::Descriptor::ShortEvent(e) => {
                    record.title = format!("{:?}", e.event_name);
                    record.summary = format!("{:?}", e.text);
                }
                psi::Descriptor::Content(c) => {
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

fn demux<S, E>(pctx: Arc<Mutex<RefCell<Context>>>, s: S) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    s.map_err(|e| info!("err {}: {:#?}", line!(), e))
        .for_each(move |packet| {
            let guard = pctx.lock().unwrap();
            let ctx = guard.borrow();
            let ret = if let Some(tx) = ctx.psi_processors.get(&packet.pid) {
                Some(tx.clone().send(packet))
            } else if let Some(tx) = ctx.pes_processors.get(&packet.pid) {
                Some(tx.clone().send(packet))
            } else {
                None
            };
            ret.map_err(|e| info!("err {}: {}", line!(), e)).map(|_| ())
        })
}

fn pat_processor<S, E>(
    pctx: Arc<Mutex<RefCell<Context>>>,
    s: S,
) -> impl Stream<Error = failure::Error>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s).and_then(move |bytes| {
        let bytes = &bytes[..];
        let table_id = bytes[0];
        match table_id {
            psi::PROGRAM_ASSOCIATION_SECTION => {
                let guard = pctx.lock().unwrap();
                let mut ctx = guard.borrow_mut();
                let pas = psi::ProgramAssociationSection::parse(bytes)?;
                for (program_number, pid) in pas.program_association {
                    if program_number != 0 {
                        // not network pid
                        let (tx, rx) = channel(1);
                        tokio::spawn(
                            pmt_processor(pctx.clone(), rx)
                                .for_each(|_| ok(()))
                                .map_err(|e| info!("err {}: {:#?}", line!(), e)),
                        );
                        ctx.psi_processors.insert(pid, tx);
                    }
                }
                return Ok(());
            }
            _ => unreachable!(),
        }
    })
}

fn pmt_processor<S, E>(
    pctx: Arc<Mutex<RefCell<Context>>>,
    s: S,
) -> impl Stream<Error = failure::Error>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s).and_then(move |bytes| {
        let bytes = &bytes[..];
        let table_id = bytes[0];
        match table_id {
            psi::TS_PROGRAM_MAP_SECTION => {
                let guard = pctx.lock().unwrap();
                let mut ctx = guard.borrow_mut();
                let pms = psi::TSProgramMapSection::parse(bytes)?;
                debug!("program map section: {:#?}", pms);
                for si in pms.stream_info.iter() {
                    ctx.stream_types.insert(si.stream_type);
                    match si.stream_type {
                        ts::MPEG2_VIDEO_STREAM
                        | ts::PES_PRIVATE_STREAM
                        | ts::ADTS_AUDIO_STREAM
                        | ts::H264_VIDEO_STREAM => {
                            // todo
                            let (tx, rx) = channel(1);
                            tokio::spawn(
                                pes_processor(rx)
                                    .for_each(|_| ok(()))
                                    .map_err(|e| info!("err {}: {:#?}", line!(), e)),
                            );
                            ctx.pes_processors.insert(si.elementary_pid, tx);
                        }
                        _ => {}
                    };
                }
                return Ok(());
            }
            _ => unreachable!(),
        }
    })
}

fn eit_processor<S, E>(
    pctx: Arc<Mutex<RefCell<Context>>>,
    s: S,
) -> impl Stream<Error = failure::Error>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s).and_then(move |bytes| {
        let guard = pctx.lock().unwrap();
        let mut ctx = guard.borrow_mut();
        let bytes = &bytes[..];
        let table_id = bytes[0];
        match table_id {
            n if 0x4e <= n && n <= 0x6f => {
                let eit = psi::EventInformationSection::parse(bytes)?;
                return match ctx.service_id {
                    Some(id) if id == eit.service_id => process_eit(&mut ctx.events, eit),
                    _ => Ok(()),
                };
            }
            _ => unreachable!(),
        }
    })
}

fn sdt_processor<S, E>(
    pctx: Arc<Mutex<RefCell<Context>>>,
    s: S,
) -> impl Stream<Error = failure::Error>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s).and_then(move |bytes| {
        let guard = pctx.lock().unwrap();
        let mut ctx = guard.borrow_mut();
        let bytes = &bytes[..];
        let table_id = bytes[0];
        match table_id {
            n if psi::SELF_STREAM_TABLE_ID == n => {
                let sdt = psi::ServiceDescriptionSection::parse(bytes)?;
                if ctx.service_id.is_none() && !sdt.services.is_empty() {
                    ctx.service_id = Some(sdt.services[0].service_id);
                }
                return Ok(());
            }
            _ => {
                unreachable!("bug");
            }
        }
    })
}

fn pes_processor<S, E>(s: S) -> impl Stream<Error = failure::Error>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    pes::Buffer::new(s).and_then(move |bytes| {
        match pes::PESPacket::parse(&bytes[..]) {
            Ok(pes) => {
                if pes.stream_id == 0b10111101 {
                    // info!("pes private stream1 {:?}", pes);
                } else {
                    debug!("pes {:#?}", pes);
                }
            }
            Err(e) => {
                info!("pes parse error: {:#?}", e);
                info!("raw bytes : {:#?}", bytes);
            }
        }
        ok(())
    })
}

fn main() {
    env_logger::init();

    tokio::run(lazy(|| {
        let pctx = Arc::new(Mutex::new(RefCell::new(Context::new())));
        {
            let guard = pctx.lock().unwrap();
            let mut ctx = guard.borrow_mut();

            // pat
            let (tx, rx) = channel(1);
            tokio::spawn(
                pat_processor(pctx.clone(), rx)
                    .for_each(|_| ok(()))
                    .map_err(|e| info!("err {}: {:#?}", line!(), e)),
            );
            ctx.psi_processors.insert(PAT_PID, tx);

            // eit
            for pid in EIT_PIDS.iter() {
                let (tx, rx) = channel(1);
                tokio::spawn(
                    eit_processor(pctx.clone(), rx)
                        .for_each(|_| ok(()))
                        .map_err(|e| info!("err {}: {:#?}", line!(), e)),
                );
                ctx.psi_processors.insert(*pid, tx);
            }

            // sdt
            let (tx, rx) = channel(1);
            tokio::spawn(
                sdt_processor(pctx.clone(), rx)
                    .for_each(|_| ok(()))
                    .map_err(|e| info!("err {}: {:#?}", line!(), e)),
            );
            ctx.psi_processors.insert(psi::SDT_PID, tx);
        }
        let decoder = FramedRead::new(stdin(), ts::TSPacketDecoder::new());
        demux(pctx.clone(), decoder).then(move |_| {
            let guard = pctx.lock().unwrap();
            let mut ctx = guard.borrow_mut();
            info!("types: {:#?}", ctx.stream_types);
            info!("descriptors: {:#?}", ctx.descriptors);
            let pids = ctx
                .pes_processors
                .keys()
                .chain(ctx.psi_processors.keys())
                .cloned()
                .collect::<HashSet<_>>();
            info!("proceeded {:#?}", pids);
            info!("pids: {:#?}", ctx.pids.difference(&pids));
            for e in ctx.events.values() {
                println!("{}", serde_json::to_string(e).unwrap());
            }
            // close channels
            ctx.psi_processors.clear();
            ctx.pes_processors.clear();
            ok(())
        })
    }));
}
