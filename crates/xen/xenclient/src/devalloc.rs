use bit_vec::BitVec;

const DEVICE_COUNT: usize = 4096;
const BYTE_COUNT: usize = DEVICE_COUNT / 8;

pub struct DeviceIdAllocator {
    states: BitVec,
    cursor: u32,
}

impl Default for DeviceIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceIdAllocator {
    pub fn new() -> Self {
        Self {
            states: BitVec::from_elem(DEVICE_COUNT, false),
            cursor: 0,
        }
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != BYTE_COUNT + 4 {
            return None;
        }

        let cursor = bytes[0] as u32
            | ((bytes[1] as u32) << 8)
            | ((bytes[2] as u32) << 16)
            | ((bytes[3] as u32) << 24);
        let slice = &bytes[4..BYTE_COUNT + 4];
        if slice.len() != BYTE_COUNT {
            return None;
        }
        let states = BitVec::from_bytes(slice);

        Some(Self { states, cursor })
    }

    pub fn allocate(&mut self) -> Option<u32> {
        let start = self.cursor;
        loop {
            let id = self.cursor;
            let value = self.states.get(self.cursor as usize)?;

            self.cursor = (self.cursor + 1) % DEVICE_COUNT as u32;

            if !value {
                self.states.set(id as usize, true);
                return Some(id);
            }

            if self.cursor == start {
                return None;
            }
        }
    }

    pub fn release(&mut self, id: u32) {
        self.states.set(id as usize, false);
    }

    pub fn count_free(&mut self) -> u32 {
        self.states.count_zeros() as u32
    }

    pub fn serialize(&mut self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(BYTE_COUNT + 4);
        bytes.push((self.cursor & 0xff) as u8);
        bytes.push(((self.cursor >> 8) & 0xff) as u8);
        bytes.push(((self.cursor >> 16) & 0xff) as u8);
        bytes.push(((self.cursor >> 24) & 0xff) as u8);
        bytes.extend_from_slice(&self.states.to_bytes());
        bytes
    }
}
