const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut n = 0;
        while n < 8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04c11db7;
            } else {
                crc <<= 1;
            }
            n += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

#[allow(dead_code)]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffffffff;
    for x in data.iter() {
        let i = ((crc >> 24) as u8) ^ x;
        crc = CRC32_TABLE[i as usize] ^ (crc << 8);
    }
    return crc;
}
