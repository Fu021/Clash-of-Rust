use memmap2::MmapMut;
use std::io;

pub struct SourceText(Storage);

enum Storage {
    Heap(Vec<u8>),
    Mapped { data: MmapMut, used: usize },
}

impl SourceText {
    pub(super) const MAPPING_THRESHOLD: usize = 128 * 1024;

    pub(super) fn new(capacity: usize) -> io::Result<Self> {
        let storage = if capacity < Self::MAPPING_THRESHOLD {
            Storage::Heap(Vec::with_capacity(capacity))
        } else {
            Storage::Mapped {
                data: MmapMut::map_anon(capacity)?,
                used: 0,
            }
        };
        Ok(Self(storage))
    }

    fn as_bytes(&self) -> &[u8] {
        match &self.0 {
            Storage::Heap(data) => data,
            Storage::Mapped { data, used } => &data[..*used],
        }
    }

    pub(super) fn len(&self) -> usize {
        self.as_bytes().len()
    }

    pub fn as_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.as_bytes())
    }

    pub(super) fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let length = self.len() + bytes.len();
        match &mut self.0 {
            Storage::Heap(data) if length < Self::MAPPING_THRESHOLD => {
                data.extend_from_slice(bytes);
                return Ok(());
            }
            Storage::Mapped { data, used } if length <= data.len() => {
                data[*used..length].copy_from_slice(bytes);
                *used = length;
                return Ok(());
            }
            _ => {}
        }
        let used = self.len();
        let mut data = MmapMut::map_anon(length.next_power_of_two())?;
        data[..used].copy_from_slice(self.as_bytes());
        data[used..length].copy_from_slice(bytes);
        self.0 = Storage::Mapped { data, used: length };
        Ok(())
    }
}
