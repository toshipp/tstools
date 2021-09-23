use anyhow::{bail, Error};

#[derive(Debug)]
pub enum Descriptor<'a> {
    ShortEventDescriptor(ShortEventDescriptor<'a>),
    ExtendedEventDescriptor(ExtendedEventDescriptor<'a>),
    ContentDescriptor(ContentDescriptor),
    StreamIdentifierDescriptor(StreamIdentifierDescriptor),
    Unsupported(UnsupportedDescriptor<'a>),
}

#[derive(Debug)]
pub struct ShortEventDescriptor<'a> {
    pub iso_639_language_code: String,
    pub event_name: &'a [u8],
    pub text: &'a [u8],
}

impl<'a> ShortEventDescriptor<'a> {
    fn parse(bytes: &[u8]) -> Result<ShortEventDescriptor<'_>, Error> {
        let tag = bytes[0];
        if tag != 0x4d {
            bail!("invalid tag");
        }
        let iso_639_language_code = String::from_utf8(bytes[2..5].to_vec())?;
        let event_name_length = usize::from(bytes[5]);
        let event_name = &bytes[6..6 + event_name_length];
        let text;
        {
            let bytes = &bytes[6 + event_name_length..];
            let text_length = usize::from(bytes[0]);
            text = &bytes[1..1 + text_length];
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
    pub item_description: &'a [u8],
    pub item: &'a [u8],
}

impl ExtendedEventDescriptorItem<'_> {
    fn parse(bytes: &[u8]) -> Result<(ExtendedEventDescriptorItem<'_>, usize), Error> {
        let item_description_length = usize::from(bytes[0]);
        let item_description = &bytes[1..1 + item_description_length];
        let item_length;
        let item;
        {
            let bytes = &bytes[1 + item_description_length..];
            item_length = usize::from(bytes[0]);
            item = &bytes[1..1 + item_length];
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
    pub text: &'a [u8],
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
        let text = &bytes[1..1 + text_length];
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

#[derive(Debug)]
pub enum Genre {
    News,
    Sports,
    Information,
    Drama,
    Music,
    Variety,
    Movies,
    Animation,
    Documentary,
    Theatre,
    Hobby,
    Welfare,
    Reserved,
    Extention,
    Others,
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
                0x6 => Genre::Movies,
                0x7 => Genre::Animation,
                0x8 => Genre::Documentary,
                0x9 => Genre::Theatre,
                0xa => Genre::Hobby,
                0xb => Genre::Welfare,
                0xc | 0xd => Genre::Reserved,
                0xe => Genre::Extention,
                0xf => Genre::Others,
                _ => unreachable!(),
            };
            items.push(genre);
            bytes = &bytes[2..];
        }
        Ok(ContentDescriptor { items })
    }
}

#[derive(Debug)]
pub struct StreamIdentifierDescriptor {
    pub component_tag: u8,
}

impl StreamIdentifierDescriptor {
    fn parse(bytes: &[u8]) -> Result<StreamIdentifierDescriptor, Error> {
        let tag = bytes[0];
        if tag != 0x52 {
            bail!("invalid tag");
        }
        let component_tag = bytes[2];
        Ok(StreamIdentifierDescriptor { component_tag })
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
            0x4d => Descriptor::ShortEventDescriptor(ShortEventDescriptor::parse(bytes)?),
            0x4e => Descriptor::ExtendedEventDescriptor(ExtendedEventDescriptor::parse(bytes)?),
            0x54 => Descriptor::ContentDescriptor(ContentDescriptor::parse(bytes)?),
            0x52 => {
                Descriptor::StreamIdentifierDescriptor(StreamIdentifierDescriptor::parse(bytes)?)
            }
            _ => Descriptor::Unsupported(UnsupportedDescriptor::parse(bytes)?),
        };
        return Ok((descriptor, descriptor_length + 2));
    }
}
