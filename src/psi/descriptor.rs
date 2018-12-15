use failure::Error;
use failure::{bail, format_err};

use std::error;
use std::fmt;

use jisx0213;

use std::char;
use std::iter::Iterator;

#[derive(Copy, Clone, Debug)]
enum Code {
    Kanji,
    Eisu,
    Hiragana,
    Katakana,
    MosaicA,
    MosaicB,
    MosaicC,
    MosaicD,
    ProportionalEisu,
    ProportionalHiragana,
    ProportionalKatakana,
    JISX0201,
    JISGokanKanji1,
    JISGokanKanji2,
    TsuikaKigou,
    DRCS(u8),
    Macro,
}

enum Invocation {
    Lock(Code),
    Single(Code, Code),
}

struct AribDecodeError {}

impl fmt::Display for AribDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "decode failed")
    }
}

impl fmt::Debug for AribDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "decode failed")
    }
}

impl error::Error for AribDecodeError {}

impl Code {
    fn from_termination(f: u8) -> Code {
        match f {
            0x42 => Code::Kanji,
            0x4a => Code::Eisu,
            0x30 => Code::Hiragana,
            0x31 => Code::Katakana,
            0x32 => Code::MosaicA,
            0x33 => Code::MosaicB,
            0x34 => Code::MosaicC,
            0x35 => Code::MosaicD,
            0x036 => Code::ProportionalEisu,
            0x037 => Code::ProportionalHiragana,
            0x038 => Code::ProportionalKatakana,
            0x49 => Code::JISX0201,
            0x39 => Code::JISGokanKanji1,
            0x3a => Code::JISGokanKanji2,
            0x3b => Code::TsuikaKigou,
            0x40..=0x4f => Code::DRCS(f - 0x40),
            0x70 => Code::Macro,
            _ => unreachable!(),
        }
    }

    fn decode<I: Iterator<Item = u8>>(&self, iter: &mut I, out: &mut String) -> Result<(), Error> {
        match self {
            Code::Kanji | Code::JISGokanKanji1 => {
                let code_point = 0x10000
                    | (u32::from(iter.next().ok_or(AribDecodeError {})?) << 8)
                    | u32::from(iter.next().ok_or(AribDecodeError {})?);
                let chars = jisx0213::code_point_to_chars(code_point).ok_or(AribDecodeError {})?;
                out.extend(chars);
            }
            Code::Eisu | Code::ProportionalEisu => {
                out.push(char::from(iter.next().ok_or(AribDecodeError {})?))
            }
            Code::Hiragana | Code::ProportionalHiragana => {
                let c = match iter.next().ok_or(AribDecodeError {})? {
                    code_point @ 0x21..=0x73 => {
                        println!("h cp: {}", code_point);
                        0x3041 + u32::from(code_point) - 0x21
                    }
                    0x77 => 0x309d,
                    0x78 => 0x309e,
                    0x79 => 0x30fc,
                    0x7a => 0x3002,
                    0x7b => 0x300c,
                    0x7c => 0x300d,
                    0x7d => 0x3001,
                    0x7e => 0x30fb,
                    _ => unreachable!(),
                };
                println!("h {}", unsafe { char::from_u32_unchecked(c) });
                out.push(unsafe { char::from_u32_unchecked(c) });
            }
            Code::Katakana | Code::ProportionalKatakana => {
                let c = match iter.next().ok_or(AribDecodeError {})? {
                    code_point @ 0x21..=0x76 => 0x30a1 + u32::from(code_point) - 0x21,
                    0x77 => 0x30fd,
                    0x78 => 0x30fe,
                    0x79 => 0x30fc,
                    0x7a => 0x3002,
                    0x7b => 0x300c,
                    0x7c => 0x300d,
                    0x7d => 0x3001,
                    0x7e => 0x30fb,
                    _ => unreachable!(),
                };
                out.push(unsafe { char::from_u32_unchecked(c) });
            }
            Code::MosaicA | Code::MosaicB | Code::MosaicC | Code::MosaicD => unimplemented!(),
            Code::JISX0201 => {
                let c = 0xff61 + u32::from(iter.next().ok_or(AribDecodeError {})?) - 0x21;
                out.push(unsafe { char::from_u32_unchecked(c) });
            }
            Code::JISGokanKanji2 => {
                let code_point = 0x20000
                    | (u32::from(iter.next().ok_or(AribDecodeError {})?) << 8)
                    | u32::from(iter.next().ok_or(AribDecodeError {})?);
                out.extend(jisx0213::code_point_to_chars(code_point).ok_or(AribDecodeError {})?);
            }
            Code::TsuikaKigou => println!(
                "tsuikakigou {:x} {:x}",
                iter.next().ok_or(AribDecodeError {})?,
                iter.next().ok_or(AribDecodeError {})?
            ),
            Code::DRCS(_n) => unimplemented!(),
            Code::Macro => unimplemented!(),
        }
        //println!("{:?} {}", self, out);
        Ok(())
    }
}

struct AribDecoder {
    gl: Invocation,
    gr: Invocation,
    g: [Code; 4],
}

const ESC: u8 = 0x1b;
const LS0: u8 = 0xf;
const LS1: u8 = 0xe;
const LS2: u8 = 0x6e;
const LS3: u8 = 0x6f;
const LS1R: u8 = 0x7e;
const LS2R: u8 = 0x7d;
const LS3R: u8 = 0x7c;
const SS2: u8 = 0x19;
const SS3: u8 = 0x1d;

// 1byte parameter
const PAPF: u8 = 0x16;
const APS: u8 = 0x1c;

// leading byte is 0x20, it takes more 1 byte.
const COL: u8 = 0x90;

// 1byte pearameter
const POL: u8 = 0x93;

const SZX: u8 = 0x8b;
const FLC: u8 = 0x91;

// leading byte is 0x20, it takes more 1 byte.
const CDC: u8 = 0x92;

const WMM: u8 = 0x94;

// 2 byte params
const TIME: u8 = 0x9d;

const MACRO: u8 = 0x95;
const RPC: u8 = 0x98;
const HLC: u8 = 0x97;
const CSI: u8 = 0x9b;

impl AribDecoder {
    fn new() -> AribDecoder {
        AribDecoder {
            gl: Invocation::Lock(Code::Kanji),
            gr: Invocation::Lock(Code::Hiragana),
            g: [Code::Kanji, Code::Eisu, Code::Hiragana, Code::Katakana],
        }
    }

    fn decode(&mut self, input: &[u8]) -> Result<String, Error> {
        let mut iter = input.iter().cloned().peekable();
        let mut string = String::new();
        while let Some(&b) = iter.peek() {
            if self.is_control(b) {
                self.set_state(&mut iter)?
            } else {
                let code = if b < 0x80 {
                    match self.gl {
                        // todo
                        Invocation::Lock(code) => code,
                        Invocation::Single(code, p) => {
                            self.gl = Invocation::Lock(p);
                            code
                        }
                    }
                } else {
                    match self.gr {
                        // todo
                        Invocation::Lock(code) => code,
                        Invocation::Single(code, p) => {
                            self.gl = Invocation::Lock(p);
                            code
                        }
                    }
                };
                let mut iter = (&mut iter).map(move |x| x & 0x7f);
                code.decode(&mut iter, &mut string)
                    .map_err(|_| format_err!("{}, {:?}", string, input))?;
            }
        }
        println!("{} {:?}", string, input);
        Ok(string)
    }

    fn is_control(&self, b: u8) -> bool {
        (b & 0x7f) < 0x20
    }

    fn set_state<I: Iterator<Item = u8>>(&mut self, s: &mut I) -> Result<(), Error> {
        let s0 = s.next().ok_or(AribDecodeError {})?;
        match s0 {
            LS0 => self.gl = Invocation::Lock(self.g[0]),
            LS1 => self.gl = Invocation::Lock(self.g[1]),
            ESC => {
                let s1 = s.next().ok_or(AribDecodeError {})?;
                match s1 {
                    LS2 => self.gl = Invocation::Lock(self.g[2]),
                    LS3 => self.gl = Invocation::Lock(self.g[3]),
                    LS1R => self.gr = Invocation::Lock(self.g[1]),
                    LS2R => self.gr = Invocation::Lock(self.g[2]),
                    LS3R => self.gr = Invocation::Lock(self.g[3]),
                    0x28..=0x2b => {
                        let pos = usize::from(s1 - 0x28);
                        let s2 = s.next().ok_or(AribDecodeError {})?;
                        let code = if s2 == 0x20 {
                            // DRCS
                            let s3 = s.next().ok_or(AribDecodeError {})?;
                            Code::from_termination(s3)
                        } else {
                            Code::from_termination(s2)
                        };
                        self.g[pos] = code;
                    }
                    0x24 => {
                        let s2 = s.next().ok_or(AribDecodeError {})?;
                        match s2 {
                            0x28 => {
                                let s3 = s.next().ok_or(AribDecodeError {})?;
                                if s3 != 0x20 {
                                    unreachable!();
                                }
                                let s4 = s.next().ok_or(AribDecodeError {})?;
                                self.g[0] = Code::from_termination(s4);
                            }
                            0x29..=0x2b => {
                                let s3 = s.next().ok_or(AribDecodeError {})?;
                                let pos = usize::from(s2 - 0x28);
                                let code = if s3 == 0x20 {
                                    // DRCS
                                    let s4 = s.next().ok_or(AribDecodeError {})?;
                                    Code::from_termination(s4)
                                } else {
                                    Code::from_termination(s3)
                                };
                                self.g[pos] = code;
                            }
                            _ => self.g[0] = Code::from_termination(s2),
                        }
                    }
                    _ => {
                        unreachable!();
                    }
                }
            }
            SS2 => {
                // multiple single shift?
                let prev = match self.gl {
                    Invocation::Lock(p) => p,
                    Invocation::Single(_, p) => p,
                };
                self.gl = Invocation::Single(self.g[2], prev);
            }
            SS3 => {
                let prev = match self.gl {
                    Invocation::Lock(p) => p,
                    Invocation::Single(_, p) => p,
                };
                self.gl = Invocation::Single(self.g[3], prev);
            }
            0x00..=0x1f => {
                // c0
                //todo
                match s0 {
                    PAPF | APS => {
                        s.next();
                    }
                    _ => {}
                }
            }
            0x80..=0x9f => {
                // c1
                println!("c1 {}", s0);
                match s0 {
                    COL | POL | SZX | FLC | CDC | WMM | RPC | HLC => {
                        if s.next().ok_or(AribDecodeError {})? == 0x20 {
                            s.next();
                        }
                    }
                    TIME | MACRO | CSI => {
                        unimplemented!();
                    }
                    _ => {}
                }
            }
            _ => {
                // non control
                unreachable!()
            }
        }
        Ok(())
    }
}

pub struct AribString<'a>(&'a [u8]);

impl<'a> fmt::Debug for AribString<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut decoder = AribDecoder::new();
        let s = decoder.decode(self.0);
        write!(f, "{:?}", s)
    }
}

#[derive(Debug)]
pub enum Descriptor<'a> {
    ShortEvent(ShortEventDescriptor<'a>),
    ExtendedEvent(ExtendedEventDescriptor<'a>),
    Content(ContentDescriptor),
    Unsupported(UnsupportedDescriptor<'a>),
}

#[derive(Debug)]
pub struct ShortEventDescriptor<'a> {
    pub iso_639_language_code: String,
    pub event_name: AribString<'a>,
    pub text: AribString<'a>,
}

impl<'a> ShortEventDescriptor<'a> {
    fn parse(bytes: &[u8]) -> Result<ShortEventDescriptor<'_>, Error> {
        let tag = bytes[0];
        if tag != 0x4d {
            bail!("invalid tag");
        }
        let iso_639_language_code = String::from_utf8((&bytes[2..5]).to_vec())?;
        let event_name_length = usize::from(bytes[5]);
        let event_name = AribString(&bytes[6..6 + event_name_length]);
        let text;
        {
            let bytes = &bytes[6 + event_name_length..];
            let text_length = usize::from(bytes[0]);
            text = AribString(&bytes[1..1 + text_length]);
        }
        Ok(ShortEventDescriptor {
            iso_639_language_code,
            event_name,
            text,
        })
    }
}

#[derive(Debug)]
pub struct ExtendedEventDescriptorItem<'a> {
    pub item_description: AribString<'a>,
    pub item: AribString<'a>,
}

impl<'a> ExtendedEventDescriptorItem<'a> {
    fn parse(bytes: &[u8]) -> Result<(ExtendedEventDescriptorItem<'_>, usize), Error> {
        let item_description_length = usize::from(bytes[0]);
        let item_description = AribString(&bytes[1..1 + item_description_length]);
        let item_length;
        let item;
        {
            let bytes = &bytes[1 + item_description_length..];
            item_length = usize::from(bytes[0]);
            item = AribString(&bytes[1..1 + item_length]);
        }
        Ok((
            ExtendedEventDescriptorItem {
                item_description,
                item,
            },
            2 + item_description_length + item_length,
        ))
    }
}

#[derive(Debug)]
pub struct ExtendedEventDescriptor<'a> {
    pub descriptor_number: u8,
    pub last_descriptor_number: u8,
    pub iso_639_language_code: String,
    pub items: Vec<ExtendedEventDescriptorItem<'a>>,
    pub text: AribString<'a>,
}

impl<'a> ExtendedEventDescriptor<'a> {
    fn parse(bytes: &[u8]) -> Result<ExtendedEventDescriptor<'_>, Error> {
        let tag = bytes[0];
        if tag != 0x4e {
            bail!("invalid tag");
        }
        let descriptor_number = bytes[2] >> 4;
        let last_descriptor_number = bytes[2] & 0xf;
        let iso_639_language_code = String::from_utf8((&bytes[3..6]).to_vec())?;
        let length_of_items = usize::from(bytes[6]);
        let mut items = Vec::new();
        {
            let mut bytes = &bytes[7..7 + length_of_items];
            while bytes.len() > 0 {
                let (item, n) = ExtendedEventDescriptorItem::parse(bytes)?;
                items.push(item);
                bytes = &bytes[n..];
            }
        }
        let bytes = &bytes[7 + length_of_items..];
        let text_length = usize::from(bytes[0]);
        let text = AribString(&bytes[1..1 + text_length]);
        Ok(ExtendedEventDescriptor {
            descriptor_number,
            last_descriptor_number,
            iso_639_language_code,
            items,
            text,
        })
    }
}

#[derive(Debug)]
pub struct ContentDescriptor {
    pub items: Vec<Genre>,
}

pub enum Genre {
    News,
    Sports,
    Information,
    Drama,
    Music,
    Variety,
    Cinema,
    Anime,
    Documentary,
    Theater,
    Hoby,
    Weal,
    Reserve,
    Extended,
    Other,
}

impl fmt::Display for Genre {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Genre::News => "ニュース/報道",
            Genre::Sports => "スポーツ",
            Genre::Information => "情報/ワイドショー",
            Genre::Drama => "ドラマ",
            Genre::Music => "音楽",
            Genre::Variety => "バラエティ",
            Genre::Cinema => "映画",
            Genre::Anime => "アニメ/特撮",
            Genre::Documentary => "ドキュメンタリー/教養",
            Genre::Theater => "劇場/公演",
            Genre::Hoby => "趣味/教育",
            Genre::Weal => "福祉",
            Genre::Reserve => "予備",
            Genre::Extended => "拡張",
            Genre::Other => "その他",
        };
        write!(f, "{}", s)
    }
}

impl fmt::Debug for Genre {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self as &dyn fmt::Display).fmt(f)
    }
}

impl ContentDescriptor {
    fn parse(bytes: &[u8]) -> Result<ContentDescriptor, Error> {
        let tag = bytes[0];
        if tag != 0x54 {
            bail!("invalid tag");
        }
        let length = usize::from(bytes[1]);
        let mut bytes = &bytes[2..2 + length];
        let mut items = Vec::new();
        while bytes.len() > 0 {
            let content_nibble_level_1 = bytes[0] >> 4;
            let genre = match content_nibble_level_1 {
                0x0 => Genre::News,
                0x1 => Genre::Sports,
                0x2 => Genre::Information,
                0x3 => Genre::Drama,
                0x4 => Genre::Music,
                0x5 => Genre::Variety,
                0x6 => Genre::Cinema,
                0x7 => Genre::Anime,
                0x8 => Genre::Documentary,
                0x9 => Genre::Theater,
                0xa => Genre::Hoby,
                0xb => Genre::Weal,
                0xc | 0xd => Genre::Reserve,
                0xe => Genre::Extended,
                0xf => Genre::Other,
                _ => unreachable!(),
            };
            items.push(genre);
            bytes = &bytes[2..];
        }
        Ok(ContentDescriptor { items })
    }
}

#[derive(Debug)]
pub struct UnsupportedDescriptor<'a> {
    pub descriptor_tag: u8,
    pub data: &'a [u8],
}

impl<'a> UnsupportedDescriptor<'a> {
    fn parse(bytes: &[u8]) -> Result<UnsupportedDescriptor<'_>, Error> {
        let descriptor_tag = bytes[0];
        let length = usize::from(bytes[1]);
        Ok(UnsupportedDescriptor {
            descriptor_tag,
            data: &bytes[2..2 + length],
        })
    }
}

impl<'a> Descriptor<'a> {
    pub fn parse(bytes: &[u8]) -> Result<(Descriptor<'_>, usize), Error> {
        check_len!(bytes.len(), 2);
        let descriptor_tag = bytes[0];
        let descriptor_length = usize::from(bytes[1]);
        let descriptor = match descriptor_tag {
            0x4d => Descriptor::ShortEvent(ShortEventDescriptor::parse(bytes)?),
            0x4e => Descriptor::ExtendedEvent(ExtendedEventDescriptor::parse(bytes)?),
            0x54 => Descriptor::Content(ContentDescriptor::parse(bytes)?),
            _ => Descriptor::Unsupported(UnsupportedDescriptor::parse(bytes)?),
        };
        return Ok((descriptor, descriptor_length + 2));
    }
}
