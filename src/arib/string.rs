use failure::format_err;
use failure::Error;
use std::char;
use std::error;
use std::fmt;

#[derive(Copy, Clone, Debug)]
enum Code {
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
            0x4a => Code::Alnum,
            0x30 => Code::Hiragana,
            0x31 => Code::Katakana,
            0x32 => Code::MosaicA,
            0x33 => Code::MosaicB,
            0x34 => Code::MosaicC,
            0x35 => Code::MosaicD,
            0x036 => Code::ProportionalAlnum,
            0x037 => Code::ProportionalHiragana,
            0x038 => Code::ProportionalKatakana,
            0x49 => Code::JISX0201,
            0x39 => Code::JISGokanKanji1,
            0x3a => Code::JISGokanKanji2,
            0x3b => Code::Symbol,
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
                let chars = jisx0213::code_point_to_chars(code_point)
                    .ok_or(format_err!("unknown cp: {:x}", code_point))?;
                out.extend(chars);
            }
            Code::Alnum | Code::ProportionalAlnum => {
                out.push(char::from(iter.next().ok_or(AribDecodeError {})?))
            }
            Code::Hiragana | Code::ProportionalHiragana => {
                let c = match iter.next().ok_or(AribDecodeError {})? {
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
            Code::Symbol => println!(
                "symbol {:x} {:x}",
                iter.next().ok_or(AribDecodeError {})?,
                iter.next().ok_or(AribDecodeError {})?
            ),
            Code::DRCS(_n) => unimplemented!(),
            Code::Macro => unimplemented!(),
        }
        Ok(())
    }
}

pub struct AribDecoder {
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
    pub fn new() -> AribDecoder {
        AribDecoder {
            gl: Invocation::Lock(Code::Kanji),
            gr: Invocation::Lock(Code::Hiragana),
            g: [Code::Kanji, Code::Alnum, Code::Hiragana, Code::Katakana],
        }
    }

    pub fn decode<'a, I: Iterator<Item = &'a u8>>(&mut self, iter: I) -> Result<String, Error> {
        let mut iter = iter.cloned().peekable();
        let mut string = String::new();
        while let Some(&b) = iter.peek() {
            if self.is_control(b) {
                self.set_state(&mut iter, &mut string)?
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
                code.decode(&mut iter, &mut string).map_err(|e| {
                    println!("partial decoded {}", string);
                    e
                })?;
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
                println!("c0 {}", s0);
                match s0 {
                    PAPF | APS => {
                        s.next();
                    }
                    _ => {}
                }
            }
            0x20 => out.push(' '),
            0x7f => {
                // DEL
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
