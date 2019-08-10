use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::PathBuf;

use failure::{bail, Error};
use futures::future::lazy;
use log::{debug, info};
use md5::{Digest, Md5};
use serde_derive::{Deserialize, Serialize};
use serde_json;
use tokio::codec::FramedRead;
use tokio::prelude::future::{err, ok, Future};
use tokio::prelude::Stream;
use tokio::runtime::Builder;

use super::common;
use super::io::path_to_async_read;
use crate::arib;
use crate::pes;
use crate::stream::cueable;
use crate::ts;

fn sync_caption<'a>(pes: &'a pes::PESPacket) -> Result<arib::caption::DataGroup<'a>, Error> {
    if let pes::PESPacketBody::NormalPESPacketBody(ref body) = pes.body {
        arib::pes::SynchronizedPESData::parse(body.pes_packet_data_byte)
            .and_then(|data| arib::caption::DataGroup::parse(data.synchronized_pes_data_byte))
    } else {
        unreachable!();
    }
}

fn async_caption<'a>(pes: &'a pes::PESPacket) -> Result<arib::caption::DataGroup<'a>, Error> {
    if let pes::PESPacketBody::DataBytes(bytes) = pes.body {
        arib::pes::AsynchronousPESData::parse(bytes)
            .and_then(|data| arib::caption::DataGroup::parse(data.asynchronous_pes_data_byte))
    } else {
        unreachable!();
    }
}

fn get_caption<'a>(pes: &'a pes::PESPacket) -> Result<arib::caption::DataGroup<'a>, Error> {
    match pes.stream_id {
        arib::pes::SYNCHRONIZED_PES_STREAM_ID => sync_caption(pes),
        arib::pes::ASYNCHRONOUS_PES_STREAM_ID => async_caption(pes),
        _ => bail!("unknown pes"),
    }
}

fn print_aa(cc: u16, hash: u128, font: &arib::caption::Font) {
    info!("cc = {}, hash = {:x}", cc, hash);
    for y in 0..font.height {
        let mut aa = String::new();
        for x in 0..font.width {
            let pos = usize::from(x) + usize::from(y) * usize::from(font.width);
            let data = font.pattern_data[pos / 4];
            let shift = 6 - (pos % 4) * 2;
            let v = (data >> shift) & 0x3;
            if v > 0 {
                aa.push_str(&format!("{}", v));
            } else {
                aa.push(' ');
            }
        }
        info!("{:?}", aa);
    }
}

#[derive(Hash, PartialEq, Eq)]
struct U128(u128);

struct U128Visitor;
impl<'de> serde::de::Visitor<'de> for U128Visitor {
    type Value = U128;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an md5 string")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u128::from_str_radix(v, 16) {
            Ok(x) => Ok(U128(x)),
            Err(e) => Err(E::custom(format!("{} can not be parsed as u128: {}", v, e))),
        }
    }
}

impl<'de> serde::Deserialize<'de> for U128 {
    fn deserialize<D>(deserializer: D) -> Result<U128, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_string(U128Visitor)
    }
}

#[derive(Deserialize)]
struct DRCSMap {
    drcs: HashMap<U128, String>,
}

struct DRCSProcessor {
    unknown: HashSet<u128>,
    drcs_map: HashMap<u128, String>,
    code_map: HashMap<u16, String>,
    handle_drcs: HandleDRCS,
}

impl DRCSProcessor {
    fn new(handle_drcs: HandleDRCS) -> DRCSProcessor {
        DRCSProcessor {
            unknown: HashSet::new(),
            drcs_map: HashMap::new(),
            code_map: HashMap::new(),
            handle_drcs: handle_drcs,
        }
    }

    fn load_map(&mut self, path: PathBuf) -> Result<(), Error> {
        let file = File::open(path)?;
        let map: DRCSMap = serde_json::from_reader(file)?;
        self.drcs_map = map.drcs.into_iter().map(|(k, v)| (k.0, v)).collect();
        Ok(())
    }

    fn process(&mut self, data: &[u8]) -> Result<(), Error> {
        let drcs = arib::caption::DrcsDataStructure::parse(data)?;
        for code in drcs.codes {
            let mut code_str = String::new();
            let mut found_font = false;
            for font in code.fonts {
                let hash = u128::from_ne_bytes(Md5::digest(font.pattern_data).into());
                match self.drcs_map.get(&hash) {
                    Some(s) => {
                        code_str.push_str(s);
                        found_font = true
                    }
                    None => {
                        if self.unknown.insert(hash) {
                            print_aa(code.character_code, hash, &font);
                        }
                        if let HandleDRCS::FailFast = self.handle_drcs {
                            bail!(
                                "unknown replacement string for cc = {}, hash = {}",
                                code.character_code,
                                hash
                            );
                        }
                    }
                }
            }
            if found_font {
                self.code_map.insert(code.character_code, code_str);
            } else {
                self.code_map
                    .insert(code.character_code, String::from("\u{fffd}"));
            }
        }
        Ok(())
    }

    fn code_map(&self) -> HashMap<u16, String> {
        self.code_map.clone()
    }

    fn clear_code_map(&mut self) {
        self.code_map.clear();
    }

    fn report_error(self) -> Result<(), Error> {
        if let HandleDRCS::ErrorExit = self.handle_drcs {
            if !self.unknown.is_empty() {
                bail!("found {} unknown drcs font", self.unknown.len());
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct Caption {
    time_sec: u64,
    time_ms: u64,
    caption: String,
}

fn dump_caption<'a>(
    data_units: &Vec<arib::caption::DataUnit<'a>>,
    offset: u64,
    drcs_processor: &mut DRCSProcessor,
) -> Result<(), Error> {
    drcs_processor.clear_code_map();

    for du in data_units {
        match &du.data_unit_parameter {
            arib::caption::DataUnitParameter::Text => {
                let mut decoder = arib::string::AribDecoder::with_caption_initialization();
                decoder.set_drcs(drcs_processor.code_map());
                let caption_string = match decoder.decode(du.data_unit_data.iter()) {
                    Ok(s) => s,
                    Err(e) => {
                        debug!("raw: {:?}", du.data_unit_data);
                        return Err(e);
                    }
                };
                if !caption_string.is_empty() {
                    let caption = Caption {
                        time_sec: offset / pes::PTS_HZ,
                        time_ms: offset % pes::PTS_HZ * 1000 / pes::PTS_HZ,
                        caption: caption_string,
                    };
                    println!("{}", serde_json::to_string(&caption)?);
                }
            }
            arib::caption::DataUnitParameter::DRCS1 => drcs_processor.process(du.data_unit_data)?,
            param => {
                debug!("unsupported data unit {:?}", param);
            }
        }
    }
    Ok(())
}

fn process_captions<S: Stream<Item = ts::TSPacket, Error = Error>>(
    pid: u16,
    base_pts: u64,
    drcs_processor: DRCSProcessor,
    s: S,
) -> impl Future<Item = (), Error = Error> {
    let caption_stream = s.filter(move |packet| packet.pid == pid);
    pes::Buffer::new(caption_stream)
        .fold(drcs_processor, move |mut drcs_processor, bytes| {
            let pes = match pes::PESPacket::parse(&bytes[..]) {
                Ok(pes) => pes,
                Err(e) => {
                    info!("pes parse error: {:?}", e);
                    return ok(drcs_processor);
                }
            };
            let offset = match pes.get_pts() {
                Some(now) => {
                    // if the caption is designated to be display
                    // before the first picture,
                    // ignore it.
                    if now < base_pts {
                        return ok(drcs_processor);
                    }
                    now - base_pts
                }
                _ => return ok(drcs_processor),
            };
            let dg = match get_caption(&pes) {
                Ok(dg) => dg,
                Err(e) => {
                    info!("retrieving caption error: {:?}", e);
                    return ok(drcs_processor);
                }
            };
            let data_units = match dg.data_group_data {
                arib::caption::DataGroupData::CaptionManagementData(ref cmd) => &cmd.data_units,
                arib::caption::DataGroupData::CaptionData(ref cd) => &cd.data_units,
            };
            if let Err(e) = dump_caption(data_units, offset, &mut drcs_processor) {
                return err(e);
            }
            ok(drcs_processor)
        })
        .then(|result| match result {
            Ok(drcs_processor) => drcs_processor.report_error(),
            Err(e) => Err(e),
        })
}

pub enum HandleDRCS {
    Ignore,
    FailFast,
    ErrorExit,
}

impl std::str::FromStr for HandleDRCS {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ignore" => Ok(HandleDRCS::Ignore),
            "fail-fast" => Ok(HandleDRCS::FailFast),
            "error-exit" => Ok(HandleDRCS::ErrorExit),
            s => bail!("unknown option: {}", s),
        }
    }
}

pub fn run(
    input: Option<PathBuf>,
    drcs_map: Option<PathBuf>,
    handle_drcs: HandleDRCS,
) -> Result<(), Error> {
    let mut drcs_processor = DRCSProcessor::new(handle_drcs);
    if let Some(path) = drcs_map {
        drcs_processor.load_map(path)?;
    }

    let proc = lazy(|| {
        path_to_async_read(input).and_then(|input| {
            let packets = FramedRead::new(input, ts::TSPacketDecoder::new());
            let cueable_packets = cueable(packets);
            common::find_main_meta(cueable_packets).and_then(|(meta, s)| {
                let packets = s.cue_up();
                let cueable_packets = cueable(packets);
                common::find_first_picture_pts(meta.video_pid, cueable_packets).and_then(
                    move |(pts, s)| {
                        let packets = s.cue_up();
                        process_captions(meta.caption_pid, pts, drcs_processor, packets)
                    },
                )
            })
        })
    });

    let rt = Builder::new().build()?;
    rt.block_on_all(proc)
}
