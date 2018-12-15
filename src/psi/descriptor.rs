use failure::bail;
use failure::Error;

use std::fmt;

use crate::arib::string::AribDecoder;

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
