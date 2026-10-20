use std::fs::OpenOptions;
use std::io::{Error, ErrorKind, Result};
use std::os::unix::io::AsRawFd;
use std::ptr;

pub const EVENT_LOG_MAGIC: u32 = 0x45564C33;
pub const EVENT_LOG_VERSION: u32 = 1;
pub const EVENT_HEADER_SIZE: usize = 64;
pub const EVENT_RECORD_SIZE: usize = 64;
pub const EVENT_PAYLOAD_SIZE: usize = 40;
pub const EVENT_LOG_CAPACITY: usize = 2 * 1024 * 1024;

#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct EventLogHeader {
    pub magic: u32,
    pub version: u32,
    pub record_count: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub flags: u32,
    pub _pad: [u8; 28],
}

impl Default for EventLogHeader {
    fn default() -> Self {
        Self {
            magic: EVENT_LOG_MAGIC,
            version: EVENT_LOG_VERSION,
            record_count: 0,
            first_seq: 0,
            last_seq: 0,
            flags: 0,
            _pad: [0; 28],
        }
    }
}

#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct EventRecord {
    pub sequence_number: u64,
    pub ns_timestamp: u64,
    pub event_type: u8,
    pub _pad: [u8; 7],
    pub payload: [u8; EVENT_PAYLOAD_SIZE],
}

impl Default for EventRecord {
    fn default() -> Self {
        Self {
            sequence_number: 0,
            ns_timestamp: 0,
            event_type: 0,
            _pad: [0; 7],
            payload: [0; EVENT_PAYLOAD_SIZE],
        }
    }
}

pub struct EventLogWriter {
    map: *mut u8,
    map_len: usize,
    fd: i32,
    header: *mut EventLogHeader,
    records: *mut EventRecord,
    max_records: usize,
    write_pos: usize,
    flush_pos: usize,
}

unsafe impl Send for EventLogWriter {}

impl EventLogWriter {
    pub fn open(path: &str, max_bytes: usize) -> Result<Self> {
        let aligned_len = (max_bytes + 4095) & !4095;
    let map_len = aligned_len + 4096;
        if aligned_len < EVENT_HEADER_SIZE + EVENT_RECORD_SIZE {
            return Err(Error::new(ErrorKind::InvalidInput, "capacity too small"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let fd = file.as_raw_fd();
        std::mem::forget(file);
        let ret = unsafe { libc::ftruncate(fd, map_len as i64) };
        if ret < 0 {
            // SAFETY: Closing file descriptor on truncate failure.
            unsafe { libc::close(fd) };
            return Err(Error::last_os_error());
        }
        let map = unsafe {
            libc::mmap(
                ptr::null_mut(),
                map_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if map == libc::MAP_FAILED {
            // SAFETY: Closing file descriptor on mmap failure.
            unsafe { libc::close(fd) };
            return Err(Error::last_os_error());
        }
        let header = map as *mut EventLogHeader;
        let records = unsafe { (map as *mut u8).add(EVENT_HEADER_SIZE) as *mut EventRecord };
        let max_records = (max_bytes.saturating_sub(EVENT_HEADER_SIZE)) / EVENT_RECORD_SIZE;
        // SAFETY: Writing initial header into freshly mapped zeroed region.
        unsafe {
            ptr::write(header, EventLogHeader::default());
        }
        Ok(Self {
            map: map as *mut u8,
            map_len,
            fd,
            header,
            records,
            max_records,
            write_pos: 0,
            flush_pos: 0,
        })
    }

    #[inline(always)]
    pub fn append(
        &mut self,
        sequence_number: u64,
        ns_timestamp: u64,
        event_type: u8,
        payload: &[u8],
    ) -> Result<u64> {
        if self.write_pos >= self.max_records {
            return Err(Error::new(ErrorKind::WriteZero, "event log full"));
        }
        let copy_len = payload.len().min(EVENT_PAYLOAD_SIZE);
        let mut record = EventRecord::default();
        record.sequence_number = sequence_number;
        record.ns_timestamp = ns_timestamp;
        record.event_type = event_type;
        if copy_len > 0 {
            record.payload[..copy_len].copy_from_slice(&payload[..copy_len]);
        }
        // SAFETY: write_pos bounds-checked against max_records above.
        unsafe {
            ptr::write(self.records.add(self.write_pos), record);
            let hdr = &mut *self.header;
            hdr.record_count = (self.write_pos + 1) as u64;
            if hdr.first_seq == 0 {
                hdr.first_seq = sequence_number;
            }
            hdr.last_seq = sequence_number;
        }
        self.write_pos += 1;
        Ok(sequence_number)
    }

    #[inline(always)]
    pub fn append_market_event(
        &mut self,
        sequence_number: u64,
        event_bytes: &[u8],
    ) -> Result<u64> {
        let ns_timestamp = if event_bytes.len() >= 8 {
            u64::from_le_bytes(event_bytes[0..8].try_into().unwrap())
        } else {
            0
        };
        let event_type = if event_bytes.len() >= 49 {
            event_bytes[48]
        } else {
            0
        };
        self.append(sequence_number, ns_timestamp, event_type, event_bytes)
    }

    pub fn flush(&mut self) -> Result<()> {
        if self.write_pos == self.flush_pos {
            return Ok(());
        }
        let dirty_start_raw = EVENT_HEADER_SIZE + self.flush_pos * EVENT_RECORD_SIZE;
        let dirty_start = dirty_start_raw & !4095;
        let dirty_end = EVENT_HEADER_SIZE + self.write_pos * EVENT_RECORD_SIZE;
        let dirty_len = dirty_end - dirty_start;
        // SAFETY: Flushing dirty mmap region to backing storage via msync.
        let ret = unsafe {
            libc::msync(
                self.map.add(dirty_start) as *mut libc::c_void,
                dirty_len,
                libc::MS_ASYNC,
            )
        };
        if ret < 0 {
            return Err(Error::last_os_error());
        }
        let hdr_dirty = unsafe {
            libc::msync(
                self.map as *mut libc::c_void,
                EVENT_HEADER_SIZE,
                libc::MS_ASYNC,
            )
        };
        if hdr_dirty < 0 {
            return Err(Error::last_os_error());
        }
        self.flush_pos = self.write_pos;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.flush()?;
        // SAFETY: Synchronous msync to guarantee durability.
        let ret = unsafe {
            libc::msync(
                self.map as *mut libc::c_void,
                self.map_len,
                libc::MS_SYNC,
            )
        };
        if ret < 0 {
            return Err(Error::last_os_error());
        }
        Ok(())
    }

    #[inline(always)]
    pub fn record_count(&self) -> u64 {
        self.write_pos as u64
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.max_records
    }
}

impl Drop for EventLogWriter {
    fn drop(&mut self) {
        let _ = self.sync();
        // SAFETY: Unmapping shared memory region and closing file descriptor.
        unsafe {
            libc::munmap(self.map as *mut libc::c_void, self.map_len);
            libc::close(self.fd);
        }
    }
}

pub struct EventLogReader {
    map: *const u8,
    map_len: usize,
    fd: i32,
    header: EventLogHeader,
    records: *const EventRecord,
    read_pos: usize,
    total_records: usize,
}

unsafe impl Send for EventLogReader {}

impl EventLogReader {
    pub fn open(path: &str) -> Result<Self> {
        let file = OpenOptions::new().read(true).open(path)?;
        let fd = file.as_raw_fd();
        std::mem::forget(file);
        let file_len = unsafe { libc::lseek(fd, 0, libc::SEEK_END) } as usize;
        if file_len < EVENT_HEADER_SIZE {
            // SAFETY: Closing fd on undersized file.
            unsafe { libc::close(fd) };
            return Err(Error::new(ErrorKind::InvalidData, "file too small"));
        }
        unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
        let map = unsafe {
            libc::mmap(
                ptr::null_mut(),
                file_len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd,
                0,
            )
        };
        if map == libc::MAP_FAILED {
            // SAFETY: Closing fd on mmap failure.
            unsafe { libc::close(fd) };
            return Err(Error::last_os_error());
        }
        // SAFETY: Reading header from mapped region bounds-checked above.
        let header = unsafe { ptr::read(map as *const EventLogHeader) };
        if header.magic != EVENT_LOG_MAGIC {
            // SAFETY: Unmapping and closing on invalid magic.
            unsafe {
                libc::munmap(map, file_len);
                libc::close(fd);
            }
            return Err(Error::new(ErrorKind::InvalidData, "bad event log magic"));
        }
        let total_records = header.record_count as usize;
        let records = unsafe { (map as *const u8).add(EVENT_HEADER_SIZE) as *const EventRecord };
        Ok(Self {
            map: map as *const u8,
            map_len: file_len,
            fd,
            header,
            records,
            read_pos: 0,
            total_records,
        })
    }

    pub fn header(&self) -> &EventLogHeader {
        &self.header
    }

    pub fn total_records(&self) -> usize {
        self.total_records
    }

    pub fn reset(&mut self) {
        self.read_pos = 0;
    }

    pub fn next_record(&mut self) -> Option<&EventRecord> {
        if self.read_pos >= self.total_records {
            return None;
        }
        // SAFETY: read_pos bounds-checked against total_records from header.
        let record = unsafe { &*self.records.add(self.read_pos) };
        self.read_pos += 1;
        Some(record)
    }

    pub fn replay<F>(&mut self, mut on_record: F) -> Result<u64>
    where
        F: FnMut(&EventRecord) -> Result<()>,
    {
        self.reset();
        let mut count = 0u64;
        while let Some(rec) = self.next_record() {
            on_record(rec)?;
            count += 1;
        }
        Ok(count)
    }
}

impl Drop for EventLogReader {
    fn drop(&mut self) {
        // SAFETY: Unmapping read-only memory region and closing file descriptor.
        unsafe {
            libc::munmap(self.map as *mut libc::c_void, self.map_len);
            libc::close(self.fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_record_alignment() {
        assert_eq!(std::mem::align_of::<EventLogHeader>(), 64);
        assert_eq!(std::mem::size_of::<EventLogHeader>(), 64);
        assert_eq!(std::mem::align_of::<EventRecord>(), 64);
        assert_eq!(std::mem::size_of::<EventRecord>(), 64);
    }

    #[test]
    fn event_log_write_read_roundtrip() {
        let path = "/tmp/aw_v3_event_log_test.bin";
        let _ = std::fs::remove_file(path);
        {
            let mut writer = EventLogWriter::open(path, 64 * 1024).unwrap();
            for i in 0..100u64 {
                let payload = [i as u8; EVENT_PAYLOAD_SIZE];
                writer.append(i + 1, i * 1000, 1, &payload).unwrap();
            }
            writer.sync().unwrap();
            assert_eq!(writer.record_count(), 100);
        }
        {
            let mut reader = EventLogReader::open(path).unwrap();
            assert_eq!(reader.total_records(), 100);
            assert_eq!(reader.header().first_seq, 1);
            assert_eq!(reader.header().last_seq, 100);
            let mut count = 0u64;
            reader.replay(|rec| {
                count += 1;
                assert_eq!(rec.sequence_number, count);
                assert_eq!(rec.ns_timestamp, (count - 1) * 1000);
                assert_eq!(rec.event_type, 1);
                Ok(())
            }).unwrap();
            assert_eq!(count, 100);
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn event_log_full_capacity() {
        let path = "/tmp/aw_v3_event_log_full_test.bin";
        let _ = std::fs::remove_file(path);
        let mut writer = EventLogWriter::open(path, EVENT_HEADER_SIZE + EVENT_RECORD_SIZE * 4).unwrap();
        assert_eq!(writer.capacity(), 4);
        for i in 0..4u64 {
            writer.append(i + 1, 0, 0, &[]).unwrap();
        }
        assert!(writer.append(5, 0, 0, &[]).is_err());
        let _ = std::fs::remove_file(path);
    }
}
