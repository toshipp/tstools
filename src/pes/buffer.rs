use failure::Error;

use crate::ts;

#[derive(Debug)]
enum BufferState {
    Initial,
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

    pub fn feed<F: FnMut(&[u8]) -> Result<(), Error>>(
        &mut self,
        packet: &ts::TSPacket,
        mut f: F,
    ) -> Result<(), Error> {
        if packet.transport_error_indicator {
            return Ok(());
        }

        if packet.data_bytes.is_none() {
            bail!("no data");
        }

        let mut ret = Ok(());
        if packet.payload_unit_start_indicator {
            if let BufferState::Buffering(_) = self.state {
                if self.buf.len() > 0 {
                    ret = f(&self.buf[..]);
                }
            }
            self.state = BufferState::FindingLength;
            self.counter = packet.continuity_counter;
            self.buf.clear();
        } else {
            match self.state {
                BufferState::Initial => {
                    // seen partial packet
                    return Ok(());
                }
                BufferState::Proceeded => {
                    // already finished
                    return Ok(());
                }
                _ => {
                    if self.counter == packet.continuity_counter {
                        // duplicate packet
                        return Ok(());
                    } else if (self.counter + 1) % 16 == packet.continuity_counter {
                        self.counter = packet.continuity_counter;
                    } else {
                        self.state = BufferState::Initial;
                        bail!("pes packet discontinued");
                    }
                }
            }
        }

        self.buf.extend_from_slice(packet.data_bytes.unwrap());

        if let BufferState::FindingLength = self.state {
            if self.buf.len() < 6 {
                return ret;
            } else {
                let pes_packet_length = (u16::from(self.buf[4]) << 8) | u16::from(self.buf[5]);
                if pes_packet_length == 0 {
                    // TODO
                    self.state = BufferState::Buffering(0);
                } else {
                    self.state = BufferState::Buffering(pes_packet_length + 6);
                }
            }
        }
        if let BufferState::Buffering(length) = self.state {
            if length > 0 && self.buf.len() >= (length as usize) {
                self.state = BufferState::Proceeded;
                return ret.and(f(&self.buf[..length as usize]));
            }
        }
        return ret;
    }
}
