use std::char;
use std::collections::HashMap;

use failure;
use failure_derive::Fail;
use log::trace;

#[derive(Debug)]
enum Charset {
    Kanji,
    Alnum,
    Hiragana,
    Katakana,
    MosaicA,
    MosaicB,
    MosaicC,
    MosaicD,
    ProportionalAlnum,
    ProportionalHiragana,
    ProportionalKatakana,
    JISX0201,
    JISGokanKanji1,
    JISGokanKanji2,
    Symbol,
    DRCS(u8),
    Macro,
}

enum DesignatePos {
    G0 = 0,
    G1 = 1,
    G2 = 2,
    G3 = 3,
}

enum InvokePos {
    GL,
    GR,
}

trait State {
    fn designate(&mut self, dst: DesignatePos, cs: Charset);
    fn lock(&mut self, dst: InvokePos, src: DesignatePos);
    fn single(&mut self, src: DesignatePos);
}

impl Charset {
    fn decode<I: Iterator<Item = u8>>(
        &self,
        iter: &mut I,
        out: &mut String,
        drcs_map: &HashMap<u16, String>,
        state: &mut State,
    ) -> Result<(), failure::Error> {
        macro_rules! next {
            () => {
                iter.next().ok_or(Error::MalformedShortBytes)?
            };
        }
        match self {
            Charset::Kanji => {
                let code_point = (u16::from(next!()) << 8) | u16::from(next!());
                if code_point < 0x7500 {
                    let code_point = 0x10000 | u32::from(code_point);
                    let chars = jisx0213::code_point_to_chars(code_point)
                        .ok_or(Error::UnknownCodepoint(code_point, String::from("kanji")))?;
                    out.extend(chars);
                } else {
                    out.push(arib_symbols::code_point_to_char(code_point).ok_or(
                        Error::UnknownCodepoint(code_point as u32, String::from("kanji")),
                    )?);
                }
            }
            Charset::JISGokanKanji1 => {
                let code_point = 0x10000 | (u32::from(next!()) << 8) | u32::from(next!());
                let chars = jisx0213::code_point_to_chars(code_point).ok_or(
                    Error::UnknownCodepoint(code_point, String::from("jis gokan 1")),
                )?;
                out.extend(chars);
            }
            Charset::Alnum | Charset::ProportionalAlnum => out.push(char::from(next!())),
            Charset::Hiragana | Charset::ProportionalHiragana => {
                let c = match next!() {
                    code_point @ 0x21..=0x73 => 0x3041 + u32::from(code_point) - 0x21,
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
                out.push(unsafe { char::from_u32_unchecked(c) });
            }
            Charset::Katakana | Charset::ProportionalKatakana => {
                let c = match next!() {
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
            Charset::MosaicA | Charset::MosaicB | Charset::MosaicC | Charset::MosaicD => {
                return Err(Error::UnimplementedCharset(String::from("mosaic")).into());
            }
            Charset::JISX0201 => {
                let c = 0xff61 + u32::from(next!()) - 0x21;
                out.push(unsafe { char::from_u32_unchecked(c) });
            }
            Charset::JISGokanKanji2 => {
                let code_point = 0x20000 | (u32::from(next!()) << 8) | u32::from(next!());
                out.extend(jisx0213::code_point_to_chars(code_point).ok_or(
                    Error::UnknownCodepoint(code_point, String::from("jis gokan 2")),
                )?);
            }
            Charset::Symbol => {
                let cp = (u16::from(next!()) << 8) | u16::from(next!());
                out.push(
                    arib_symbols::code_point_to_char(cp)
                        .ok_or(Error::UnknownCodepoint(cp as u32, String::from("symbol")))?,
                );
            }
            Charset::DRCS(n) => {
                let cc = if *n == 0 {
                    (u16::from(next!()) << 8) | u16::from(next!())
                } else {
                    (u16::from(0x40 + *n) << 8) | u16::from(next!())
                };
                match drcs_map.get(&cc) {
                    Some(s) => out.push_str(s),
                    None => {
                        return Err(
                            Error::UnknownCodepoint(cc as u32, format!("drcs({})", *n)).into()
                        );
                    }
                }
            }
            Charset::Macro => {
                let n = next!();
                match n {
                    0x60 => {
                        state.designate(DesignatePos::G0, Charset::Kanji);
                        state.designate(DesignatePos::G1, Charset::Alnum);
                        state.designate(DesignatePos::G2, Charset::Hiragana);
                        state.designate(DesignatePos::G3, Charset::Macro);
                        state.lock(InvokePos::GL, DesignatePos::G0);
                        state.lock(InvokePos::GR, DesignatePos::G2);
                    }
                    0x61 => {
                        state.designate(DesignatePos::G0, Charset::Kanji);
                        state.designate(DesignatePos::G1, Charset::Katakana);
                        state.designate(DesignatePos::G2, Charset::Hiragana);
                        state.designate(DesignatePos::G3, Charset::Macro);
                        state.lock(InvokePos::GL, DesignatePos::G0);
                        state.lock(InvokePos::GR, DesignatePos::G2);
                    }
                    _ => {
                        return Err(Error::UnknownCodepoint(n as u32, String::from("macro")).into());
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Fail)]
enum Error {
    #[fail(display = "unknown code point: 0x{:x} in {:}", 0, 1)]
    UnknownCodepoint(u32, String),
    #[fail(display = "unimplemented charset: {:}", 0)]
    UnimplementedCharset(String),
    #[fail(display = "unimplemented control: 0x{:x}", 0)]
    UnimplementedControl(u8),
    #[fail(display = "malformed short bytes")]
    MalformedShortBytes,
}

pub struct AribDecoder {
    single: Option<usize>,
    gl: usize,
    gr: usize,
    g: [Charset; 4],
    drcs_map: HashMap<u16, String>,
}

// escape sequence
const LS2: u8 = 0x6e;
const LS3: u8 = 0x6f;
const LS1R: u8 = 0x7e;
const LS2R: u8 = 0x7d;
const LS3R: u8 = 0x7c;

// C0
const NUL: u8 = 0x0;
const BEL: u8 = 0x7;
const APB: u8 = 0x8;
const APF: u8 = 0x9;
const APD: u8 = 0xa;
const APU: u8 = 0xb;
const CS: u8 = 0xc;
const APR: u8 = 0xd;
const LS1: u8 = 0xe;
const LS0: u8 = 0xf;
const PAPF: u8 = 0x16;
const CAN: u8 = 0x18;
const SS2: u8 = 0x19;
const ESC: u8 = 0x1b;
const APS: u8 = 0x1c;
const SS3: u8 = 0x1d;
const RS: u8 = 0x1e;
const US: u8 = 0x1f;

const SP: u8 = 0x20;
const DEL: u8 = 0x7f;

// C1
const BKF: u8 = 0x80;
const RDF: u8 = 0x81;
const GRF: u8 = 0x82;
const YLF: u8 = 0x83;
const BLF: u8 = 0x84;
const MGF: u8 = 0x85;
const CNF: u8 = 0x86;
const WHF: u8 = 0x87;
const SSZ: u8 = 0x88; // font size small
const MSZ: u8 = 0x89; // font size middle
const NSZ: u8 = 0x8a; // font size normal
const SZX: u8 = 0x8b;
const COL: u8 = 0x90;
const FLC: u8 = 0x91;
const CDC: u8 = 0x92;
const POL: u8 = 0x93;
const WMM: u8 = 0x94;
const MACRO: u8 = 0x95;
const HLC: u8 = 0x97;
const RPC: u8 = 0x98;
const SPL: u8 = 0x99;
const STL: u8 = 0x9a;
const CSI: u8 = 0x9b;
const TIME: u8 = 0x9d;

struct StateModification {
    single: Option<usize>,
    gl: Option<usize>,
    gr: Option<usize>,
    g: [Option<Charset>; 4],
}

impl StateModification {
    fn new() -> Self {
        StateModification {
            single: None,
            gl: None,
            gr: None,
            g: [None, None, None, None],
        }
    }
}

impl State for StateModification {
    fn designate(&mut self, dst: DesignatePos, cs: Charset) {
        self.g[dst as usize] = Some(cs);
    }

    fn lock(&mut self, dst: InvokePos, src: DesignatePos) {
        match dst {
            InvokePos::GL => self.gl = Some(src as usize),
            InvokePos::GR => self.gr = Some(src as usize),
        }
    }

    fn single(&mut self, src: DesignatePos) {
        self.single = Some(src as usize)
    }
}

fn is_control(b: u8) -> bool {
    let lo = b & 0x7f;
    lo <= 0x20 || lo == 0x7f
}

fn g_set_from_termination(f: u8) -> Charset {
    match f {
        0x42 => Charset::Kanji,
        0x4a => Charset::Alnum,
        0x30 => Charset::Hiragana,
        0x31 => Charset::Katakana,
        0x32 => Charset::MosaicA,
        0x33 => Charset::MosaicB,
        0x34 => Charset::MosaicC,
        0x35 => Charset::MosaicD,
        0x36 => Charset::ProportionalAlnum,
        0x37 => Charset::ProportionalHiragana,
        0x38 => Charset::ProportionalKatakana,
        0x49 => Charset::JISX0201,
        0x39 => Charset::JISGokanKanji1,
        0x3a => Charset::JISGokanKanji2,
        0x3b => Charset::Symbol,
        _ => unreachable!(),
    }
}

fn drcs_from_termination(f: u8) -> Charset {
    match f {
        0x40..=0x4f => Charset::DRCS(f - 0x40),
        0x70 => Charset::Macro,
        _ => unreachable!(),
    }
}

impl AribDecoder {
    pub fn with_event_initialization() -> AribDecoder {
        AribDecoder {
            single: None,
            gl: 0,
            gr: 2,
            g: [
                Charset::JISGokanKanji1,
                Charset::Alnum,
                Charset::Hiragana,
                Charset::Katakana,
            ],
            drcs_map: HashMap::new(),
        }
    }

    pub fn with_caption_initialization() -> AribDecoder {
        AribDecoder {
            single: None,
            gl: 0,
            gr: 2,
            g: [
                Charset::Kanji,
                Charset::Alnum,
                Charset::Hiragana,
                Charset::Macro,
            ],
            drcs_map: HashMap::new(),
        }
    }

    pub fn set_drcs(&mut self, drcs_map: HashMap<u16, String>) {
        self.drcs_map = drcs_map;
    }

    pub fn decode<'a, I: Iterator<Item = &'a u8>>(
        mut self,
        iter: I,
    ) -> Result<String, failure::Error> {
        let mut iter = iter.cloned().peekable();
        let mut string = String::new();
        while let Some(&b) = iter.peek() {
            if is_control(b) {
                self.control(&mut iter, &mut string)?
            } else {
                let charset = if b < 0x80 {
                    match self.single {
                        Some(pos) => {
                            self.single = None;
                            &self.g[pos]
                        }
                        None => &self.g[self.gl],
                    }
                } else {
                    &self.g[self.gr]
                };
                let mut iter = (&mut iter).map(move |x| x & 0x7f);
                let mut modification = StateModification::new();
                charset.decode(&mut iter, &mut string, &self.drcs_map, &mut modification)?;
                self.apply(modification);
            }
        }
        Ok(string)
    }

    fn apply(&mut self, mut modification: StateModification) {
        if modification.single.is_some() {
            self.single = modification.single;
        }
        match modification.gl {
            Some(gl) => self.gl = gl,
            None => {}
        }
        match modification.gr {
            Some(gr) => self.gr = gr,
            None => {}
        }
        for i in 0..4 {
            match modification.g[i].take() {
                Some(cs) => self.g[i] = cs,
                None => {}
            }
        }
    }

    fn control<I: Iterator<Item = u8>>(
        &mut self,
        s: &mut I,
        out: &mut String,
    ) -> Result<(), failure::Error> {
        macro_rules! next {
            () => {
                s.next().ok_or(Error::MalformedShortBytes)?
            };
        }
        macro_rules! param1or2 {
            () => {{
                let mut v = Vec::new();
                let c = next!();
                v.push(c);
                if c == 0x20 {
                    v.push(next!());
                }
                v
            }};
        }
        let s0 = next!();
        match s0 {
            // invocation and designation
            LS0 => self.gl = 0,
            LS1 => self.gl = 1,
            ESC => {
                let s1 = next!();
                match s1 {
                    LS2 => self.gl = 2,
                    LS3 => self.gl = 3,
                    LS1R => self.gr = 1,
                    LS2R => self.gr = 2,
                    LS3R => self.gr = 3,
                    0x28..=0x2b => {
                        let pos = usize::from(s1 - 0x28);
                        let s2 = next!();
                        let code = if s2 == 0x20 {
                            // DRCS
                            let s3 = next!();
                            drcs_from_termination(s3)
                        } else {
                            g_set_from_termination(s2)
                        };
                        trace!("{}: g[{}] = {:?}", line!(), pos, code);
                        self.g[pos] = code;
                    }
                    0x24 => {
                        let s2 = next!();
                        match s2 {
                            0x28 => {
                                // DRCS
                                let s3 = next!();
                                if s3 != 0x20 {
                                    unreachable!();
                                }
                                let s4 = next!();
                                let code = drcs_from_termination(s4);
                                trace!("{}: g[0] = {:?}", line!(), code);
                                self.g[0] = code;
                            }
                            0x29..=0x2b => {
                                let s3 = next!();
                                let pos = usize::from(s2 - 0x28);
                                let code = if s3 == 0x20 {
                                    // DRCS
                                    let s4 = next!();
                                    drcs_from_termination(s4)
                                } else {
                                    g_set_from_termination(s3)
                                };
                                trace!("{}: g[{}] = {:?}", line!(), pos, code);
                                self.g[pos] = code;
                            }
                            _ => {
                                let code = g_set_from_termination(s2);
                                trace!("{}: g[0] = {:?}", line!(), code);
                                self.g[0] = code;
                            }
                        }
                    }
                    _ => {
                        unreachable!();
                    }
                }
            }
            SS2 => self.single = Some(2),
            SS3 => self.single = Some(3),

            // C0
            NUL => {
                // receiver can ignore this.
            }
            BEL => {
                out.push('\x07');
            }
            APB => {
                // retract cursor
                out.push('\x08');
            }
            APF => {
                trace!("APF");
                // advance cursor
                out.push('\t');
            }
            APD => {
                // down cursor
                out.push('\n');
            }
            APU => {
                // up cursor
                trace!("up cursor");
            }
            APR => {
                out.push('\r');
            }
            PAPF => {
                let x = next!();
                trace!("PAPF {}", x);
                for _ in 0..x {
                    out.push('\t');
                }
            }
            APS => {
                let x = next!();
                let y = next!();
                trace!("APS {} {}", x, y);
                // todo
                out.push('\n');
            }
            CS => {
                trace!("clear display");
            }
            CAN => {
                trace!("cancel");
            }
            RS => {
                trace!("begin data header");
            }
            US => {
                trace!("begin data unit");
            }
            SP => out.push(' '),
            DEL => {
                trace!("del");
            }

            // C1
            BKF | RDF | GRF | YLF | BLF | MGF | CNF | WHF => {
                trace!("color: {}", s0);
            }
            COL => {
                let param = param1or2!();
                trace!("COL {:?}", param);
            }
            POL => {
                let param = next!();
                trace!("POL {}", param);
            }
            SSZ | MSZ | NSZ => {
                trace!("font size: {}", s0);
            }
            SZX => {
                let param = next!();
                trace!("font size param: {}", param);
            }
            FLC => {
                let param = next!();
                trace!("FLC {}", param);
            }
            CDC => {
                let param = param1or2!();
                trace!("CDC {:?}", param);
            }
            WMM => {
                let param = next!();
                trace!("WMM {:?}", param);
            }
            TIME => {
                let mut seq = Vec::new();
                let c = next!();
                seq.push(c);
                match c {
                    0x20 | 0x28 => {
                        seq.push(next!());
                    }
                    0x29 => loop {
                        let c = next!();
                        seq.push(c);
                        if c >= 0x40 {
                            break;
                        }
                    },
                    _ => unreachable!(),
                }
                trace!("TIME {:?}", seq);
            }
            MACRO => {
                return Err(Error::UnimplementedControl(s0).into());
            }
            RPC => {
                return Err(Error::UnimplementedControl(s0).into());
            }
            STL | SPL => {
                return Err(Error::UnimplementedControl(s0).into());
            }
            HLC => {
                let param = next!();
                trace!("HLC {}", param);
            }
            CSI => {
                let mut seq = Vec::new();
                loop {
                    let c = next!();
                    seq.push(c);
                    if c >= 0x40 {
                        break;
                    }
                }
                trace!("CSI {:?}", seq);
            }
            0xa0 => {}
            0xff => {}

            x => trace!("unknown control: {}", x),
        }
        Ok(())
    }
}
