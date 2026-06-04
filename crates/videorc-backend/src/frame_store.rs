use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct StoredFrame<P> {
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub pixel_format: P,
    pub bytes: Vec<u8>,
    pub captured_at: Instant,
}

pub type FrameHandle<P> = Arc<StoredFrame<P>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStoreStats {
    pub buffer_count: u64,
    pub bytes_retained: u64,
    pub frames_dropped: u64,
    pub buffer_allocations: u64,
}

#[derive(Debug)]
pub struct FrameStore<P> {
    latest: Option<FrameHandle<P>>,
    spare_buffers: Vec<Vec<u8>>,
    max_spare_buffers: usize,
    frames_replaced: u64,
    buffer_allocations: u64,
}

impl<P> Default for FrameStore<P> {
    fn default() -> Self {
        Self::new(1)
    }
}

impl<P> FrameStore<P> {
    pub fn new(max_spare_buffers: usize) -> Self {
        Self {
            latest: None,
            spare_buffers: Vec::new(),
            max_spare_buffers,
            frames_replaced: 0,
            buffer_allocations: 0,
        }
    }

    pub fn latest(&self) -> Option<FrameHandle<P>> {
        self.latest.clone()
    }

    pub fn checkout_buffer(&mut self, byte_len: usize) -> Vec<u8> {
        let mut buffer = self.spare_buffers.pop().unwrap_or_else(|| {
            self.buffer_allocations = self.buffer_allocations.saturating_add(1);
            Vec::with_capacity(byte_len)
        });
        if buffer.capacity() < byte_len {
            self.buffer_allocations = self.buffer_allocations.saturating_add(1);
            buffer = Vec::with_capacity(byte_len);
        }
        buffer.resize(byte_len, 0);
        buffer
    }

    pub fn publish(
        &mut self,
        sequence: u64,
        width: u32,
        height: u32,
        pixel_format: P,
        captured_at: Instant,
        bytes: Vec<u8>,
    ) -> FrameHandle<P> {
        if let Some(previous) = self.latest.take() {
            self.frames_replaced = self.frames_replaced.saturating_add(1);
            if let Ok(previous) = Arc::try_unwrap(previous) {
                self.retain_spare_buffer(previous.bytes);
            }
        }

        let frame = Arc::new(StoredFrame {
            sequence,
            width,
            height,
            pixel_format,
            bytes,
            captured_at,
        });
        self.latest = Some(Arc::clone(&frame));
        frame
    }

    pub fn stats(&self) -> FrameStoreStats {
        let latest_bytes = self
            .latest
            .as_ref()
            .map(|frame| frame.bytes.len() as u64)
            .unwrap_or(0);
        let spare_bytes = self
            .spare_buffers
            .iter()
            .map(|buffer| buffer.capacity() as u64)
            .sum::<u64>();
        FrameStoreStats {
            buffer_count: self.latest.iter().count() as u64 + self.spare_buffers.len() as u64,
            bytes_retained: latest_bytes.saturating_add(spare_bytes),
            frames_dropped: self.frames_replaced,
            buffer_allocations: self.buffer_allocations,
        }
    }

    fn retain_spare_buffer(&mut self, mut bytes: Vec<u8>) {
        if self.spare_buffers.len() >= self.max_spare_buffers {
            return;
        }
        bytes.clear();
        self.spare_buffers.push(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestPixelFormat {
        Rgba,
    }

    #[test]
    fn newest_frame_wins_and_old_frames_are_dropped() {
        let mut store = FrameStore::new(1);
        let first = store.checkout_buffer(4);
        store.publish(1, 1, 1, TestPixelFormat::Rgba, Instant::now(), first);
        let second = store.checkout_buffer(4);
        store.publish(2, 1, 1, TestPixelFormat::Rgba, Instant::now(), second);

        let latest = store.latest().expect("latest frame");

        assert_eq!(latest.sequence, 2);
        assert_eq!(store.stats().frames_dropped, 1);
    }

    #[test]
    fn retained_store_memory_is_bounded_after_warmup() {
        let mut store = FrameStore::new(1);

        for sequence in 1..=10 {
            let buffer = store.checkout_buffer(1024);
            store.publish(
                sequence,
                16,
                16,
                TestPixelFormat::Rgba,
                Instant::now(),
                buffer,
            );
        }

        let stats = store.stats();
        assert_eq!(stats.buffer_count, 2);
        assert!(stats.bytes_retained <= 2048);
        assert_eq!(stats.buffer_allocations, 2);
    }

    #[test]
    fn external_handles_do_not_make_store_retention_unbounded() {
        let mut store = FrameStore::new(1);
        let mut handles = Vec::new();

        for sequence in 1..=5 {
            let buffer = store.checkout_buffer(256);
            handles.push(store.publish(
                sequence,
                8,
                8,
                TestPixelFormat::Rgba,
                Instant::now(),
                buffer,
            ));
        }

        let stats = store.stats();
        assert_eq!(stats.buffer_count, 1);
        assert_eq!(stats.bytes_retained, 256);
        assert_eq!(handles.len(), 5);
    }
}
