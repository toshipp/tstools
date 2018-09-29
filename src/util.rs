use failure::Error;

pub fn read_u32(bytes: &[u8]) -> Result<u32, Error> {
    if bytes.len() < 4 {
        bail!("too short {}", bytes.len());
    }
    return Ok((u32::from(bytes[0]) << 24)
        | (u32::from(bytes[1]) << 16)
        | (u32::from(bytes[2]) << 8)
        | u32::from(bytes[3]));
}
