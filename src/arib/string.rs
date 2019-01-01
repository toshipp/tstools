use super::symbol;
use failure::format_err;
use failure::Error;
use log::info;
use std::char;
use std::error;
use std::fmt;

pub fn decode_to_utf8<'a, I: Iterator<Item = &'a u8>>(iter: I) -> Result<String, Error> {
    AribDecoder::new().decode(iter)
}

#[derive(Copy, Clone)]
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

impl Charset {
    fn from_termination(f: u8) -> Charset {
        match f {
            0x42 => Charset::Kanji,
            0x4a => Charset::Alnum,
            0x30 => Charset::Hiragana,
            0x31 => Charset::Katakana,
            0x32 => Charset::MosaicA,
            0x33 => Charset::MosaicB,
            0x34 => Charset::MosaicC,
            0x35 => Charset::MosaicD,
            0x036 => Charset::ProportionalAlnum,
            0x037 => Charset::ProportionalHiragana,
            0x038 => Charset::ProportionalKatakana,
            0x49 => Charset::JISX0201,
            0x39 => Charset::JISGokanKanji1,
            0x3a => Charset::JISGokanKanji2,
            0x3b => Charset::Symbol,
            0x40..=0x4f => Charset::DRCS(f - 0x40),
            0x70 => Charset::Macro,
            _ => unreachable!(),
        }
    }

    fn decode<I: Iterator<Item = u8>>(&self, iter: &mut I, out: &mut String) -> Result<(), Error> {
        macro_rules! next {
            () => {
                iter.next().ok_or(AribDecodeError {})?
            };
        }
        match self {
            Charset::Kanji | Charset::JISGokanKanji1 => {
                let code_point = 0x10000 | (u32::from(next!()) << 8) | u32::from(next!());
                let chars = jisx0213::code_point_to_chars(code_point)
                    .ok_or(format_err!("unknown cp: {:x}", code_point))?;
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
                unimplemented!()
            }
            Charset::JISX0201 => {
                let c = 0xff61 + u32::from(next!()) - 0x21;
                out.push(unsafe { char::from_u32_unchecked(c) });
            }
            Charset::JISGokanKanji2 => {
                let code_point = 0x20000 | (u32::from(next!()) << 8) | u32::from(next!());
                out.extend(jisx0213::code_point_to_chars(code_point).ok_or(AribDecodeError {})?);
            }
            Charset::Symbol => {
                let cp = (u16::from(next!()) << 8) | u16::from(next!());
                match symbol::to_char(cp) {
                    Some(c) => out.push(c),
                    None => info!("unsupported symbol {:x}", cp),
                }
            }
            Charset::DRCS(_n) => unimplemented!(),
            Charset::Macro => unimplemented!(),
        }
        Ok(())
    }
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

enum Invocation {
    Lock(Charset),
    Single(Charset, Charset),
}

impl Invocation {
    fn decode<I: Iterator<Item = u8>>(
        &mut self,
        iter: &mut I,
        out: &mut String,
    ) -> Result<(), Error> {
        match self {
            Invocation::Lock(c) => c.decode(iter, out),
            &mut Invocation::Single(now, prev) => {
                *self = Invocation::Lock(prev);
                now.decode(iter, out)
            }
        }
    }

    fn lock(&mut self, c: Charset) {
        *self = Invocation::Lock(c)
    }

    fn single(&mut self, c: Charset) {
        if let Invocation::Lock(prev) = *self {
            *self = Invocation::Single(c, prev);
        } else {
            unreachable!();
        }
    }
}

struct AribDecoder {
    gl: Invocation,
    gr: Invocation,
    g: [Charset; 4],
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

// same as CR.
const APR: u8 = 0xd;

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

// set font size to small, middle or normal, accordingly.
const SSZ: u8 = 0x88;
const MSZ: u8 = 0x89;
const NSZ: u8 = 0x8a;

impl AribDecoder {
    fn new() -> AribDecoder {
        AribDecoder {
            gl: Invocation::Lock(Charset::Kanji),
            gr: Invocation::Lock(Charset::Hiragana),
            g: [
                Charset::Kanji,
                Charset::Alnum,
                Charset::Hiragana,
                Charset::Katakana,
            ],
        }
    }

    fn decode<'a, I: Iterator<Item = &'a u8>>(mut self, iter: I) -> Result<String, Error> {
        let mut iter = iter.cloned().peekable();
        let mut string = String::new();
        while let Some(&b) = iter.peek() {
            if self.is_control(b) {
                self.set_state(&mut iter, &mut string)?
            } else {
                let charset = if b < 0x80 { &mut self.gl } else { &mut self.gr };
                let mut iter = (&mut iter).map(move |x| x & 0x7f);
                charset.decode(&mut iter, &mut string)?;
            }
        }
        Ok(string)
    }

    fn is_control(&self, b: u8) -> bool {
        let lo = b & 0x7f;
        lo <= 0x20 || lo == 0x7f
    }

    fn set_state<I: Iterator<Item = u8>>(
        &mut self,
        s: &mut I,
        out: &mut String,
    ) -> Result<(), Error> {
        macro_rules! next {
            () => {
                s.next().ok_or(AribDecodeError {})?
            };
        }
        let s0 = next!();
        match s0 {
            LS0 => self.gl.lock(self.g[0]),
            LS1 => self.gl.lock(self.g[1]),
            ESC => {
                let s1 = next!();
                match s1 {
                    LS2 => self.gl.lock(self.g[2]),
                    LS3 => self.gl.lock(self.g[3]),
                    LS1R => self.gr.lock(self.g[1]),
                    LS2R => self.gr.lock(self.g[2]),
                    LS3R => self.gr.lock(self.g[3]),
                    0x28..=0x2b => {
                        let pos = usize::from(s1 - 0x28);
                        let s2 = next!();
                        let code = if s2 == 0x20 {
                            // DRCS
                            let s3 = next!();
                            Charset::from_termination(s3)
                        } else {
                            Charset::from_termination(s2)
                        };
                        self.g[pos] = code;
                    }
                    0x24 => {
                        let s2 = next!();
                        match s2 {
                            0x28 => {
                                let s3 = next!();
                                if s3 != 0x20 {
                                    unreachable!();
                                }
                                let s4 = next!();
                                self.g[0] = Charset::from_termination(s4);
                            }
                            0x29..=0x2b => {
                                let s3 = next!();
                                let pos = usize::from(s2 - 0x28);
                                let code = if s3 == 0x20 {
                                    // DRCS
                                    let s4 = next!();
                                    Charset::from_termination(s4)
                                } else {
                                    Charset::from_termination(s3)
                                };
                                self.g[pos] = code;
                            }
                            _ => self.g[0] = Charset::from_termination(s2),
                        }
                    }
                    _ => {
                        unreachable!();
                    }
                }
            }
            SS2 => self.gl.single(self.g[2]),
            SS3 => self.gl.single(self.g[3]),
            0x00..=0x1f => {
                // c0
                match s0 {
                    PAPF | APS => {
                        s.next();
                    }
                    APR => {
                        out.push('\r');
                    }
                    _ => {
                        info!("c0 {}", s0);
                    }
                }
            }
            0x20 => out.push(' '),
            0x7f => {
                // DEL
            }
            0x80..=0x9f => {
                // c1
                match s0 {
                    COL | POL | SZX | FLC | CDC | WMM | RPC | HLC => {
                        if next!() == 0x20 {
                            s.next();
                        }
                    }
                    TIME | MACRO | CSI => {
                        unimplemented!();
                    }
                    SSZ | MSZ | NSZ => {
                        // ignore font size change
                    }
                    _ => {
                        info!("c1 {}", s0);
                    }
                }
            }
            0xa0 => {}
            0xff => {}
            _ => {
                // non control
                unreachable!()
            }
        }
        Ok(())
    }
}
