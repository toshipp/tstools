use failure::Error;

use ts;

#[derive(Debug)]
enum BufferState {
    Initial,
    Skipping(u8),
    FindingLength,
    Buffering(u16),
    Proceeded,
}

#[derive(Debug)]
pub struct Buffer {
    state: BufferState,
    counter: u8,
    buf: Vec<u8>,
}

impl Buffer {
    pub fn new() -> Buffer {
        Buffer {
            state: BufferState::Initial,
            counter: 0,
            buf: Vec::new(),
        }
    }
    pub fn feed(&mut self, packet: &ts::TSPacket) -> Result<Option<&[u8]>, Error> {
        if packet.transport_error_indicator {
            return Ok(None);
        }
        if packet.data_bytes.is_none() {
            bail!("malformed psi packet, no data")
        }
        let mut bytes = packet.data_bytes.unwrap();

        if packet.payload_unit_start_indicator {
            let pointer_field = bytes[0];
            self.state = BufferState::Skipping(pointer_field + 1);
            self.counter = packet.continuity_counter;
            self.buf.clear();
        } else {
            match self.state {
                BufferState::Initial => {
                    // seen partial section
                    return Ok(None);
                }
                BufferState::Proceeded => {
                    // already completed
                    return Ok(None);
                }
                _ => {
                    if self.counter == packet.continuity_counter {
                        // duplicate packet
                        return Ok(None);
                    } else if (self.counter + 1) % 16 == packet.continuity_counter {
                        self.counter = packet.continuity_counter;
                    } else {
                        self.state = BufferState::Initial;
                        bail!("psi packet discontinued");
                    }
                }
            }
        }

        if let BufferState::Skipping(n) = self.state {
            if bytes.len() < (n as usize) {
                self.state = BufferState::Skipping(n - (bytes.len() as u8));
                return Ok(None);
            } else {
                self.state = BufferState::FindingLength;
                bytes = &bytes[n as usize..];
            }
        }

        self.buf.extend_from_slice(bytes);

        if let BufferState::FindingLength = self.state {
            if self.buf.len() < 3 {
                return Ok(None);
            } else {
                let section_length = (u16::from(self.buf[1] & 0xf) << 8) | u16::from(self.buf[2]);
                self.state = BufferState::Buffering(section_length + 3);
            }
        }
        if let BufferState::Buffering(length) = self.state {
            if self.buf.len() >= (length as usize) {
                self.state = BufferState::Proceeded;
                return Ok(Some(&self.buf[..length as usize]));
            }
        }
        return Ok(None);
    }
}
