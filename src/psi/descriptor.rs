use failure::Error;

#[derive(Debug)]
pub enum Descriptor<'a> {
    Unsupported(u8, &'a [u8]),
}

impl<'a> Descriptor<'a> {
    pub fn parse(bytes: &[u8]) -> Result<(Descriptor, usize), Error> {
        check_len!(bytes.len(), 2);
        let descriptor_tag = bytes[0];
        let descriptor_length = usize::from(bytes[1]);
        return Ok((
            Descriptor::Unsupported(descriptor_tag, &bytes[2..2 + descriptor_length]),
            descriptor_length + 2,
        ));
    }
}
