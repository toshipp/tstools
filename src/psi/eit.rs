extern crate chrono;

use self::chrono::offset::{FixedOffset, TimeZone};
use self::chrono::{DateTime, Duration};

use failure::Error;

use util;

use psi::Descriptor;

#[derive(Debug)]
pub struct Event<'a> {
    pub event_id: u16,
    pub start_time: Option<DateTime<FixedOffset>>,
    pub duration: Option<Duration>,
    pub running_status: u8,
    pub free_ca_mode: bool,

    //0x4d short event
    //0x4e exntended event desc
    //0x50 component desc
    //0x54 content desc
    //0xc4 audit component desc
    //0xc7 data contents desc.
    //0xd6 event group desc
    pub descriptors: Vec<Descriptor<'a>>,
}

#[derive(Debug)]
pub enum ScheduleType {
    SelfNow,
    OtherNow,
    SelfFuture,
    OtherFuture,
}

#[derive(Debug)]
pub struct EventInformationSection<'a> {
    pub table_id: u8,
    pub section_syntax_indicator: u8,
    pub service_id: u16,
    pub version_number: u8,
    pub current_next_indicator: u8,
    pub section_number: u8,
    pub last_section_number: u8,
    pub transport_stream_id: u16,
    pub original_network_id: u16,
    pub segment_last_section_number: u8,
    pub last_table_id: u8,
    pub events: Vec<Event<'a>>,
    pub crc_32: u32,

    _raw_bytes: &'a [u8],
    pub schedule_type: ScheduleType,
}

impl<'a> Event<'a> {
    fn parse(bytes: &[u8]) -> Result<(Event, usize), Error> {
        check_len!(bytes.len(), 12);
        let event_id = (u16::from(bytes[0]) << 8) | u16::from(bytes[1]);
        let start_time = Event::parse_datetime(&bytes[2..7])?;
        let duration = Event::parse_hms(&bytes[7..10])?.map(|(h, m, s)| {
            Duration::seconds(i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(s))
        });
        let running_status = bytes[10] >> 5;
        let free_ca_mode = (bytes[10] >> 4) & 1 > 0;
        let descriptors_loop_length = (usize::from(bytes[10] & 0xf) << 8) | usize::from(bytes[11]);
        check_len!(bytes.len() - 12, descriptors_loop_length);
        let mut bytes = &bytes[12..descriptors_loop_length + 12];
        let mut descriptors = Vec::new();
        while bytes.len() > 0 {
            let (desc, n) = Descriptor::parse(bytes)?;
            descriptors.push(desc);
            bytes = &bytes[n..];
        }
        Ok((
            Event {
                event_id,
                start_time,
                duration,
                running_status,
                free_ca_mode,
                descriptors,
            },
            descriptors_loop_length + 12,
        ))
    }

    fn parse_datetime(bytes: &[u8]) -> Result<Option<DateTime<FixedOffset>>, Error> {
        if (&bytes[..5]).iter().all(|x| *x == 0xff) {
            return Ok(None);
        }
        // Date part is lower 16 bits of MJD.
        let mjd = (u32::from(bytes[0]) << 8) | u32::from(bytes[1]);
        // +1 is from mjd and jd offset (12h), and utc and jst offset (9h).
        let jd = mjd + 2400000 + 1;
        let (y, m, d) = Event::jd_to_gregorian(jd);

        // Time part is JST BCD.
        let (hh, mm, ss) = Event::parse_hms(&bytes[2..])?.unwrap();

        Ok(Some(
            FixedOffset::east(9 * 3600).ymd(y as i32, m, d).and_hms(
                u32::from(hh),
                u32::from(mm),
                u32::from(ss),
            ),
        ))
    }

    fn jd_to_gregorian(jd: u32) -> (u32, u32, u32) {
        let y = 4716;
        let j = 1401;
        let m = 2;
        let n = 12;
        let r = 4;
        let p = 1461;
        let v = 3;
        let u = 5;
        let s = 153;
        let w = 2;
        let b = 274277;
        let c = 38;

        let f = jd + j + (4 * jd + b) / 146097 * 3 / 4 - c;
        let e = r * f + v;
        let g = (e % p) / r;
        let h = u * g + w;
        let day = (h % s) / u + 1;
        let month = (h / s + m) % n + 1;
        let year = e / p - y + (n + m - month) / n;
        (year, month, day)
    }

    fn parse_hms(bytes: &[u8]) -> Result<Option<(u8, u8, u8)>, Error> {
        // if the duration is unspecified, all bits are 1.
        if bytes[0] == 0xff && bytes[1] == 0xff && bytes[2] == 0xff {
            return Ok(None);
        }
        // It is encoded by BCD.
        let h = ((bytes[0] >> 4) * 10) + (bytes[0] & 0xf);
        let m = ((bytes[1] >> 4) * 10) + (bytes[1] & 0xf);
        let s = ((bytes[2] >> 4) * 10) + (bytes[2] & 0xf);
        Ok(Some((h, m, s)))
    }
}

impl<'a> EventInformationSection<'a> {
    pub fn parse(bytes: &[u8]) -> Result<EventInformationSection, Error> {
        let table_id = bytes[0];
        let section_syntax_indicator = bytes[1] >> 7;
        let section_length = (usize::from(bytes[1] & 0xf) << 8) | usize::from(bytes[2]);
        let service_id = (u16::from(bytes[3]) << 8) | u16::from(bytes[4]);
        let version_number = (bytes[5] >> 1) & 0x1f;
        let current_next_indicator = bytes[5] & 0x1;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let transport_stream_id = (u16::from(bytes[8]) << 8) | u16::from(bytes[9]);
        let original_network_id = (u16::from(bytes[10]) << 8) | u16::from(bytes[11]);
        let segment_last_section_number = bytes[12];
        let last_table_id = bytes[13];
        check_len!(bytes.len(), 3 + section_length);
        let mut events = Vec::new();
        {
            let mut bytes = &bytes[14..3 + section_length - 4];
            while bytes.len() > 0 {
                let (event, n) = Event::parse(bytes)?;
                events.push(event);
                bytes = &bytes[n..];
            }
        }
        let crc_32 = util::read_u32(&bytes[3 + section_length - 4..])?;
        Ok(EventInformationSection {
            table_id,
            section_syntax_indicator,
            service_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            transport_stream_id,
            original_network_id,
            segment_last_section_number,
            last_table_id,
            events,
            crc_32,
            _raw_bytes: bytes,
            schedule_type: Self::schedule_type(table_id),
        })
    }

    fn schedule_type(table_id: u8) -> ScheduleType {
        match table_id {
            0x4e => ScheduleType::SelfNow,
            0x4f => ScheduleType::OtherNow,
            n if 0x50 <= n && n <= 0x5f => ScheduleType::SelfFuture,
            n if 0x60 <= n && n <= 0x6f => ScheduleType::OtherFuture,
            _ => {
                unreachable!("invalid table_id: {}", table_id);
            }
        }
    }
}
