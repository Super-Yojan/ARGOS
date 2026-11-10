//! RTP payload to one codec frame. Incomplete fragments are not queued across frames.

/// A timestamp is late when it is older than the last displayed frame.
/// Equal timestamps are the frame we already displayed.
pub fn timestamp_is_late(timestamp: u32, last_emitted: Option<u32>) -> bool {
    let Some(last) = last_emitted else {
        return false;
    };
    if timestamp == last {
        return true;
    }
    timestamp.wrapping_sub(last) >= 0x8000_0000
}

#[derive(Default)]
pub struct H264Depacketizer {
    partial: Vec<u8>,
    open: bool,
}

impl H264Depacketizer {
    /// Completed NAL units, without start codes. A new FU-A start drops a partial unit.
    pub fn push(&mut self, payload: &[u8]) -> Vec<Vec<u8>> {
        if payload.is_empty() {
            return Vec::new();
        }
        let nal_type = payload[0] & 0x1f;
        match nal_type {
            1..=23 => {
                self.reset();
                vec![payload.to_vec()]
            }
            24 => {
                self.reset();
                stap_a(payload)
            }
            28 => self.fu_a(payload),
            _ => {
                self.reset();
                Vec::new()
            }
        }
    }

    pub fn reset(&mut self) {
        self.partial.clear();
        self.open = false;
    }

    fn fu_a(&mut self, payload: &[u8]) -> Vec<Vec<u8>> {
        if payload.len() < 2 {
            self.reset();
            return Vec::new();
        }
        let indicator = payload[0];
        let header = payload[1];
        let start = header & 0x80 != 0;
        let end = header & 0x40 != 0;
        let nal_type = header & 0x1f;
        if start {
            self.partial.clear();
            self.partial.push((indicator & 0xe0) | nal_type);
            self.partial.extend_from_slice(&payload[2..]);
            self.open = true;
        } else if self.open {
            self.partial.extend_from_slice(&payload[2..]);
        } else {
            return Vec::new();
        }
        if end && self.open {
            self.open = false;
            return vec![std::mem::take(&mut self.partial)];
        }
        Vec::new()
    }
}

fn stap_a(payload: &[u8]) -> Vec<Vec<u8>> {
    let mut units = Vec::new();
    let mut offset = 1;
    while offset + 2 <= payload.len() {
        let size = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
        offset += 2;
        if size == 0 || offset + size > payload.len() {
            break;
        }
        units.push(payload[offset..offset + size].to_vec());
        offset += size;
    }
    units
}

#[derive(Default)]
pub struct Vp8Depacketizer {
    frame: Vec<u8>,
    open: bool,
}

impl Vp8Depacketizer {
    /// Returns the raw VP8 frame when `marker` ends it. A new partition start drops a partial frame.
    pub fn push(&mut self, payload: &[u8], marker: bool) -> Option<Vec<u8>> {
        let (start, body) = vp8_payload(payload)?;
        if start {
            self.frame.clear();
            self.open = true;
        }
        if !self.open {
            return None;
        }
        self.frame.extend_from_slice(body);
        if marker {
            self.open = false;
            return Some(std::mem::take(&mut self.frame));
        }
        None
    }

    pub fn reset(&mut self) {
        self.frame.clear();
        self.open = false;
    }
}

/// `(start of partition, payload after the RTP descriptor)`.
pub fn vp8_payload(packet: &[u8]) -> Option<(bool, &[u8])> {
    if packet.is_empty() {
        return None;
    }
    let start = packet[0] & 0x10 != 0;
    let mut index = 1;
    if packet[0] & 0x80 != 0 {
        if packet.len() < 2 {
            return None;
        }
        let extension = packet[1];
        index = 2;
        if extension & 0x80 != 0 {
            if index >= packet.len() {
                return None;
            }
            let picture = packet[index];
            index += 1;
            if picture & 0x80 != 0 {
                if index >= packet.len() {
                    return None;
                }
                index += 1;
            }
        }
        if extension & 0x40 != 0 {
            index += 1;
        }
        if extension & 0x20 != 0 || extension & 0x10 != 0 {
            index += 1;
        }
    }
    if index > packet.len() {
        return None;
    }
    Some((start, &packet[index..]))
}

/// What the reorder buffer hands on: a packet in sequence order, or a gap it
/// stopped waiting for.
#[derive(Debug, PartialEq, Eq)]
pub enum Ordered<T> {
    Packet(T),
    Lost,
}

/// Puts RTP packets back in sequence order. NACK retransmissions arrive late
/// and out of order; a short wait lets them fill a gap instead of breaking
/// the picture. A gap that outlasts `wait` or `capacity` is reported as lost.
pub struct ReorderBuffer<T> {
    next: Option<u16>,
    held: std::collections::BTreeMap<u16, (std::time::Instant, T)>,
    wait: std::time::Duration,
    capacity: usize,
}

impl<T> ReorderBuffer<T> {
    pub fn new(wait: std::time::Duration, capacity: usize) -> Self {
        Self {
            next: None,
            held: std::collections::BTreeMap::new(),
            wait,
            capacity: capacity.max(1),
        }
    }

    pub fn push(&mut self, seq: u16, packet: T, now: std::time::Instant) -> Vec<Ordered<T>> {
        let next = *self.next.get_or_insert(seq);
        // Older than what was already handed on: a duplicate or too late.
        if (seq.wrapping_sub(next) as i16) < 0 {
            return self.flush(now);
        }
        self.held.entry(seq).or_insert((now, packet));
        self.flush(now)
    }

    /// Releases what is in order, and gives up on a gap that waited too long.
    pub fn flush(&mut self, now: std::time::Instant) -> Vec<Ordered<T>> {
        let mut out = Vec::new();
        let Some(mut next) = self.next else {
            return out;
        };
        loop {
            while let Some((_, packet)) = self.held.remove(&next) {
                out.push(Ordered::Packet(packet));
                next = next.wrapping_add(1);
            }
            let Some(oldest) = self.held.values().map(|(at, _)| *at).min() else {
                break;
            };
            let stale = now.saturating_duration_since(oldest) >= self.wait;
            if !stale && self.held.len() < self.capacity {
                break;
            }
            // Skip to the held packet closest after the gap.
            let Some(resume) = self
                .held
                .keys()
                .copied()
                .min_by_key(|seq| seq.wrapping_sub(next))
            else {
                break;
            };
            out.push(Ordered::Lost);
            next = resume;
        }
        self.next = Some(next);
        out
    }
}
