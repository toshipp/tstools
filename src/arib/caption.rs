use failure::bail;
use failure::Error;

#[derive(Debug)]
pub struct DataGroup<'a> {
    pub data_group_id: u8,
    pub data_group_version: u8,
    pub data_group_link_number: u8,
    pub last_data_group_link_number: u8,
    pub data_group_data: DataGroupData<'a>,
    pub crc16: u16,
}

#[derive(Debug)]
pub enum DataGroupData<'a> {
    CaptionManagementData(CaptionManagementData<'a>),
    CaptionData(CaptionData<'a>),
}

#[derive(Debug)]
pub enum TMD {
    Free,
    RealTime,
    OffsetTime,
    Reserved,
}

impl TMD {
    fn from(b: u8) -> TMD {
        match b {
            0b00 => TMD::Free,
            0b01 => TMD::RealTime,
            0b10 => TMD::OffsetTime,
            0b11 => TMD::Reserved,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub struct Time {
    h: u8,
    m: u8,
    s: u8,
    ms: u8,
}

impl Time {
    fn parse(bytes: &[u8]) -> Time {
        let h = Time::bcd2(bytes[0]);
        let m = Time::bcd2(bytes[1]);
        let s = Time::bcd2(bytes[2]);
        let ms = Time::bcd2(bytes[3]) * 10 + (bytes[4] >> 4);
        Time { h, m, s, ms }
    }

    fn bcd2(b: u8) -> u8 {
        (b >> 4) * 10 + (b & 0xf)
    }
}

#[derive(Debug)]
pub struct CaptionManagementData<'a> {
    pub tmd: TMD,
    pub otm: Option<Time>,
    pub languages: Vec<Language>,
    pub data_unit: Option<DataUnit<'a>>,
}

#[derive(Debug)]
enum TCS {
    Char8,
    UCS,
    Reseved,
}

impl TCS {
    fn from(b: u8) -> TCS {
        match b {
            0b00 => TCS::Char8,
            0b01 => TCS::UCS,
            0b10 | 0b11 => TCS::Reseved,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
enum RollupMode {
    NonRollup,
    Rollup,
    Reseved,
}

impl RollupMode {
    fn from(b: u8) -> RollupMode {
        match b {
            0b00 => RollupMode::NonRollup,
            0b01 => RollupMode::Rollup,
            0b10 | 0b11 => RollupMode::Reseved,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub struct Language {
    language_tag: u8,
    dmf: u8,
    dc: Option<u8>,
    iso_639_language_code: String,
    format: u8,
    tcs: TCS,
    rollup_mode: RollupMode,
}

#[derive(Debug)]
pub enum DataUnitParameter {
    Text,
    Geometric,
    AdditionalSound,
    DRCS1,
    DRCS2,
    ColorMap,
    BitMap,
}

impl DataUnitParameter {
    fn from(b: u8) -> DataUnitParameter {
        use DataUnitParameter::*;
        match b {
            0x20 => Text,
            0x28 => Geometric,
            0x2c => AdditionalSound,
            0x30 => DRCS1,
            0x31 => DRCS2,
            0x34 => ColorMap,
            0x35 => BitMap,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub struct DataUnit<'a> {
    pub unit_separator: u8,
    pub data_unit_parameter: DataUnitParameter,
    pub data_unit_data: &'a [u8],
}

#[derive(Debug)]
pub struct CaptionData<'a> {
    pub tmd: TMD,
    pub stm: Option<Time>,
    pub data_unit: DataUnit<'a>,
}

impl<'a> DataGroup<'a> {
    pub fn parse(bytes: &[u8]) -> Result<DataGroup, Error> {
        let data_group_id = bytes[0] >> 2;
        let data_group_version = bytes[0] & 0x3;
        let data_group_link_number = bytes[1];
        let last_data_group_link_number = bytes[2];
        let data_group_size = (usize::from(bytes[3]) << 8) | usize::from(bytes[4]);
        let data_group_data = {
            let bytes = &bytes[5..5 + data_group_size];
            if data_group_id == 0x0 || data_group_id == 0x20 {
                DataGroupData::CaptionManagementData(CaptionManagementData::parse(bytes)?)
            } else {
                DataGroupData::CaptionData(CaptionData::parse(bytes)?)
            }
        };
        let crc16 = (u16::from(bytes[5 + data_group_size]) << 8)
            | u16::from(bytes[5 + data_group_size + 1]);
        Ok(DataGroup {
            data_group_id,
            data_group_version,
            data_group_link_number,
            last_data_group_link_number,
            data_group_data,
            crc16,
        })
    }
}

impl Language {
    fn parse(mut bytes: &[u8]) -> Result<(Language, usize), Error> {
        let mut n = 5;
        let language_tag = bytes[0] >> 5;
        let dmf = bytes[0] & 0xf;
        let dc = match dmf {
            0b1100 | 0b1101 | 0b1110 => {
                let dc = bytes[1];
                bytes = &bytes[2..];
                n += 1;
                Some(dc)
            }
            _ => {
                bytes = &bytes[1..];
                None
            }
        };
        let iso_639_language_code = String::from_utf8(bytes[0..3].to_vec())?;
        let format = bytes[4] >> 4;
        let tcs = TCS::from((bytes[4] >> 2) & 0x3);
        let rollup_mode = RollupMode::from(bytes[4] & 0x3);
        Ok((
            Language {
                language_tag,
                dmf,
                dc,
                iso_639_language_code,
                format,
                tcs,
                rollup_mode,
            },
            n,
        ))
    }
}

impl<'a> CaptionManagementData<'a> {
    fn parse(mut bytes: &[u8]) -> Result<CaptionManagementData, Error> {
        let tmd = TMD::from(bytes[0] >> 6);
        let otm = match tmd {
            TMD::OffsetTime => {
                let otm = Time::parse(&bytes[1..]);
                bytes = &bytes[6..];
                Some(otm)
            }
            _ => {
                bytes = &bytes[1..];
                None
            }
        };
        let num_languages = bytes[0];
        let mut languages = Vec::new();
        bytes = &bytes[1..];
        for _ in 0..num_languages {
            let (language, n) = Language::parse(bytes)?;
            languages.push(language);
            bytes = &bytes[n..];
        }
        let data_unit_loop_length =
            (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
        let data_unit = match data_unit_loop_length {
            0 => None,
            _ => Some(DataUnit::parse(&bytes[3..3 + data_unit_loop_length])?),
        };
        Ok(CaptionManagementData {
            tmd,
            otm,
            languages,
            data_unit,
        })
    }
}

impl<'a> CaptionData<'a> {
    fn parse(mut bytes: &[u8]) -> Result<CaptionData, Error> {
        let tmd = TMD::from(bytes[0] >> 6);
        let stm = match tmd {
            TMD::RealTime | TMD::OffsetTime => {
                let stm = Time::parse(&bytes[1..]);
                bytes = &bytes[6..];
                Some(stm)
            }
            _ => {
                bytes = &bytes[1..];
                None
            }
        };
        let data_unit_loop_length =
            (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
        let data_unit = DataUnit::parse(&bytes[3..3 + data_unit_loop_length])?;
        Ok(CaptionData {
            tmd,
            stm,
            data_unit,
        })
    }
}

impl<'a> DataUnit<'a> {
    fn parse(bytes: &[u8]) -> Result<DataUnit, Error> {
        check_len!(bytes.len(), 5);
        let unit_separator = bytes[0];
        let data_unit_parameter = DataUnitParameter::from(bytes[1]);
        let data_unit_size =
            (usize::from(bytes[2]) << 16) | (usize::from(bytes[3]) << 8) | usize::from(bytes[4]);
        let data_unit_data = &bytes[5..5 + data_unit_size];
        Ok(DataUnit {
            unit_separator,
            data_unit_parameter,
            data_unit_data,
        })
    }
}
