fn index_pattern(pattern: &[u8], seq: &[u8]) -> Option<usize> {
    if pattern.len() > seq.len() {
        return None;
    }
    'outer: for i in 0..seq.len() - pattern.len() {
        for j in 0..pattern.len() {
            if seq[i + j] != pattern[j] {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

const PICTURE_START_CODE: &[u8] = &[0, 0, 1, 0];
const I_PICTURE: u8 = 1;

pub fn is_i_picture(bytes: &[u8]) -> bool {
    if let Some(index) = index_pattern(PICTURE_START_CODE, bytes) {
        let picture_header = &bytes[index..];
        if picture_header.len() >= 6 {
            let picture_coding_type = (picture_header[5] & 0x38) >> 3;
            if picture_coding_type == I_PICTURE {
                return true;
            }
        }
    }
    false
}
