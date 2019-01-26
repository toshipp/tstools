use env_logger;
use log::{debug, info};

use serde_derive::Serialize;

use failure;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;
use tokio;
use tokio::codec::FramedRead;
use tokio::io::stdin;
use tokio::prelude::future::Future;
use tokio::prelude::Stream;
use tokio::runtime::Builder;

use chrono::offset::FixedOffset;
use chrono::{DateTime, Duration};

use futures::future::lazy;

use std::collections::BTreeMap;
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

fn pat_processor<S, E>(
    pctx: Arc<Mutex<Context>>,
    mut demux_register: ts::demuxer::Register,
    s: S,
) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s)
        .for_each(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            match table_id {
                psi::PROGRAM_ASSOCIATION_SECTION => {
                    let pas = psi::ProgramAssociationSection::parse(bytes)?;
                    for (program_number, pid) in pas.program_association {
                        if program_number != 0 {
                            // not network pid
                            if let Ok(rx) = demux_register.try_register(pid) {
                                tokio::spawn(pmt_processor(
                                    pctx.clone(),
                                    demux_register.clone(),
                                    rx,
                                ));
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

fn pmt_processor<S, E>(
    pctx: Arc<Mutex<Context>>,
    mut demux_regiser: ts::demuxer::Register,
    s: S,
) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    psi::Buffer::new(s)
        .for_each(move |bytes| {
            let bytes = &bytes[..];
            let table_id = bytes[0];
            match table_id {
                psi::TS_PROGRAM_MAP_SECTION => {
                    let mut ctx = pctx.lock().unwrap();
                    let pms = psi::TSProgramMapSection::parse(bytes)?;
                    debug!("program map section: {:#?}", pms);
                    for si in pms.stream_info.iter() {
                        ctx.stream_types.insert(si.stream_type);
                        match si.stream_type {
                            ts::MPEG2_VIDEO_STREAM
                            | ts::PES_PRIVATE_STREAM
                            | ts::ADTS_AUDIO_STREAM
                            | ts::H264_VIDEO_STREAM => {
                                if let Ok(rx) = demux_regiser.try_register(si.elementary_pid) {
                                    tokio::spawn(pes_processor(rx));
                                }
                            }
                            _ => {}
                        };
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
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

fn pes_processor<S, E>(s: S) -> impl Future<Item = (), Error = ()>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    pes::Buffer::new(s)
        .for_each(move |bytes| {
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
            Ok(())
        })
        .map_err(|e| info!("err {}: {:#?}", line!(), e))
}

fn main() {
    env_logger::init();

    let proc = lazy(|| {
        let pctx = Arc::new(Mutex::new(Context::new()));
        let demuxer = ts::demuxer::Demuxer::new();
        let mut demux_register = demuxer.register();
        // pat
        tokio::spawn(pat_processor(
            pctx.clone(),
            demux_register.clone(),
            demux_register.try_register(PAT_PID).unwrap(),
        ));

        // eit
        for pid in EIT_PIDS.iter() {
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
