use anyhow::{bail, Result};

macro_rules! check_len {
    ($b:expr, $l:expr) => {
        if $b < $l {
            bail!(
                "{}: too short {}({}), expect {}({})",
                line!(),
                stringify!($b),
                $b,
                stringify!($l),
                $l
            );
        }
    };
}

pub fn read_u32(bytes: &[u8]) -> Result<u32> {
    if bytes.len() < 4 {
        bail!("too short {}", bytes.len());
    }
    return Ok((u32::from(bytes[0]) << 24)
        | (u32::from(bytes[1]) << 16)
        | (u32::from(bytes[2]) << 8)
        | u32::from(bytes[3]));
}
