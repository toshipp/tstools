use failure::Error;

#[derive(Debug)]
pub enum Descriptor {
    Descriptor(u8),
}

impl Descriptor {
    pub fn parse(bytes: &[u8]) -> Result<(Descriptor, usize), Error> {
        check_len!(bytes.len(), 2);
        let descriptor_tag = bytes[0];
        let descriptor_length = usize::from(bytes[1]);
        return Ok((
            Descriptor::Descriptor(descriptor_tag),
            descriptor_length + 2,
        ));
    }
}
