//! A shared-memory framebuffer of the MilkDrop picture, for the skin window.
//!
//! MilkDrop runs in a child process with its own OpenGL context (see
//! `super::host`), so it cannot hand the app a texture. Instead it renders
//! projectM and writes RGBA frames here, top row first; the app reads the
//! latest and uploads it as a texture to draw inside a skin's media pane.
//! It is a single-producer, single-consumer slot: the child writes a whole
//! frame then bumps the sequence, the app reads when the sequence moves, and
//! the odd torn frame while the writer is mid-copy is of no consequence to
//! a visualiser.
//!
//! The layout is a 32-byte header (magic, width, height, sequence), then the
//! RGBA bytes, row-major top-down. The sequence is atomic; the pixels are
//! plain, since a visualiser can live with a glitch.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::MmapMut;

/// The picture's magic, so a half-written or foreign mapping is refused.
const MAGIC: u32 = 0x4D445746; // "MDWF"
/// The largest picture either side will carry: a hidden skin window on a
/// Retina display.
pub const MAX_WIDTH: u32 = 1024;
pub const MAX_HEIGHT: u32 = 1024;
const HEADER: usize = 32;
const MAGIC_AT: usize = 0;
const WIDTH_AT: usize = 4;
const HEIGHT_AT: usize = 8;
const SEQ_AT: usize = 16;
/// The whole mapping's size.
pub const SIZE: usize = HEADER + (MAX_WIDTH as usize * MAX_HEIGHT as usize * 4);

/// A handle on the shared framebuffer, held by both the writer and the reader.
pub struct Frame {
    map: MmapMut,
    // The backing file is unlinked by the host when it is done; the mapping
    // keeps working until both sides drop it.
    _file: File,
}

impl Frame {
    /// Makes the file, sizes it, and maps it: the writer's side (the host
    /// makes it, the child opens it).
    pub fn create(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(SIZE as u64)?;
        // SAFETY: the file is this process's own, freshly sized to `SIZE`.
        let map = unsafe { MmapMut::map_mut(&file)? };
        let frame = Self { map, _file: file };
        frame.set_u32(MAGIC_AT, MAGIC);
        frame.set_u32(WIDTH_AT, 0);
        frame.set_u32(HEIGHT_AT, 0);
        frame.seq().store(0, Ordering::Release);
        Ok(frame)
    }

    /// Maps a file the host already made: the reader's (and the child's)
    /// side.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        if (file.metadata()?.len() as usize) < SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the MilkDrop frame buffer is too small",
            ));
        }
        // SAFETY: the file is at least `SIZE` bytes, sized by the host.
        let map = unsafe { MmapMut::map_mut(&file)? };
        let frame = Self { map, _file: file };
        if frame.get_u32(MAGIC_AT) != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the MilkDrop frame buffer is not a frame buffer",
            ));
        }
        Ok(frame)
    }

    fn get_u32(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.map[at..at + 4].try_into().unwrap_or([0; 4]))
    }

    fn set_u32(&self, at: usize, value: u32) {
        // The mapping outlives the borrow; the writer and reader each touch
        // their own end, and the sequence orders the frame.
        let ptr = self.map.as_ptr() as *mut u8;
        // SAFETY: `at..at+4` is inside the 32-byte header.
        unsafe {
            std::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), ptr.add(at), 4);
        }
    }

    /// The frame counter at the head of the mapping.
    fn seq(&self) -> &AtomicU64 {
        // SAFETY: the mapping is at least `HEADER` bytes and the base is page
        // aligned, so the eight bytes at `SEQ_AT` (16-aligned) are an aligned
        // `AtomicU64`.
        unsafe { &*(self.map.as_ptr().add(SEQ_AT) as *const AtomicU64) }
    }

    /// The picture's bytes, row-major top-down RGBA.
    fn pixels(&self) -> *mut u8 {
        // SAFETY: the mapping holds the header plus the full pixel area.
        unsafe { self.map.as_ptr().add(HEADER) as *mut u8 }
    }

    /// Writes a whole top-down RGBA frame and bumps the sequence, so a
    /// reader sees either the frame before or this one, never a half.
    /// `pixels` must be `width * height * 4` bytes; larger than the maximum
    /// is refused.
    pub fn write(&self, width: u32, height: u32, pixels: &[u8]) {
        if width == 0
            || height == 0
            || width > MAX_WIDTH
            || height > MAX_HEIGHT
            || pixels.len() < (width as usize * height as usize * 4)
        {
            return;
        }
        let bytes = width as usize * height as usize * 4;
        // SAFETY: `bytes` fits the pixel area; the reader tolerates a torn
        // frame while this copies.
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.pixels(), bytes);
        }
        self.set_u32(WIDTH_AT, width);
        self.set_u32(HEIGHT_AT, height);
        self.seq()
            .store(self.seq().load(Ordering::Relaxed) + 1, Ordering::Release);
    }

    /// The latest frame, when the sequence has moved since `cursor`: its
    /// size and a copy of its bytes, top-down RGBA. Updates the cursor past
    /// it. An empty picture (nothing written yet) comes back as `None`.
    pub fn read(&self, cursor: &mut u64) -> Option<(u32, u32, Vec<u8>)> {
        let seq = self.seq().load(Ordering::Acquire);
        if seq == *cursor {
            return None;
        }
        let (width, height) = (self.get_u32(WIDTH_AT), self.get_u32(HEIGHT_AT));
        if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            *cursor = seq;
            return None;
        }
        let bytes = width as usize * height as usize * 4;
        let mut out = vec![0u8; bytes];
        // SAFETY: `bytes` fits the pixel area.
        unsafe {
            std::ptr::copy_nonoverlapping(self.pixels(), out.as_mut_ptr(), bytes);
        }
        *cursor = seq;
        Some((width, height, out))
    }
}

// SAFETY: the frame is a single-producer, single-consumer slot; the sequence
// is atomic and the pixels tolerate a torn read, so sharing the handle
// across threads (the child writes, elsewhere reads) is sound.
unsafe impl Send for Frame {}
unsafe impl Sync for Frame {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_written_are_read_back_once() {
        let path =
            std::env::temp_dir().join(format!("fastpotify-frame-test-{}", std::process::id()));
        let writer = Frame::create(&path).unwrap();
        let reader = Frame::open(&path).unwrap();
        let mut cursor = 0;
        assert!(reader.read(&mut cursor).is_none());
        let pixels = vec![7u8; 4 * 4 * 4];
        writer.write(4, 4, &pixels);
        let (w, h, back) = reader.read(&mut cursor).unwrap();
        assert_eq!((w, h), (4, 4));
        assert_eq!(back, pixels);
        // Nothing new: nothing back.
        assert!(reader.read(&mut cursor).is_none());
        let pixels = vec![9u8; 2 * 2 * 4];
        writer.write(2, 2, &pixels);
        let (w, h, back) = reader.read(&mut cursor).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(back, pixels);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn an_oversized_or_empty_frame_is_refused() {
        let path =
            std::env::temp_dir().join(format!("fastpotify-frame-test2-{}", std::process::id()));
        let writer = Frame::create(&path).unwrap();
        let reader = Frame::open(&path).unwrap();
        let mut cursor = 0;
        writer.write(MAX_WIDTH + 1, 10, &vec![0u8; 10 * 10 * 4]);
        assert!(reader.read(&mut cursor).is_none());
        writer.write(0, 0, &[]);
        assert!(reader.read(&mut cursor).is_none());
        std::fs::remove_file(&path).unwrap();
    }
}
