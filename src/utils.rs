use indicatif::{ProgressBar, ProgressStyle};
use std::io;
use std::io::Read;

pub struct SequentialReader<'a, R> {
    source: &'a mut R,
    source_exhausted: bool,
    buffer: Vec<u8>,
    buffer_offset: usize,
    buffer_filled: usize,
}

impl<'a, R: Read> SequentialReader<'a, R> {
    pub fn new(source: &'a mut R, size: usize) -> Self {
        Self {
            source,
            source_exhausted: false,
            buffer: vec![0u8; size],
            buffer_offset: 0,
            buffer_filled: 0,
        }
    }

    pub fn read_exact(&mut self, size: usize) -> io::Result<Option<&mut [u8]>> {
        if size > self.buffer.len() {
            return Err(io::Error::other(format!(
                "Buffer is too small: tried to read {} bytes. Use the --sequential-buffer-size option to make it big enough.",
                size
            )));
        }

        assert!(self.buffer_offset <= self.buffer_filled);
        assert!(self.buffer_filled <= self.buffer.len());
        if self.buffer_filled - self.buffer_offset < size {
            if self.source_exhausted {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }

            let unread = self.buffer_filled - self.buffer_offset;
            if unread != 0 {
                self.buffer.copy_within(self.buffer_offset..self.buffer_filled, 0);
            }
            self.buffer_filled = unread;
            self.buffer_offset = 0;

            while self.buffer_filled < size && !self.source_exhausted {
                match self.source.read(&mut self.buffer[self.buffer_filled..]) {
                    Ok(0) => {
                        self.source_exhausted = true;
                    }
                    Ok(n) => {
                        self.buffer_filled += n;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            if self.buffer_filled == 0 {
                return Ok(None);
            } else if self.buffer_filled < size {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }

            self.buffer_offset = size;
            Ok(Some(&mut self.buffer[0..size]))
        } else {
            let res = &mut self.buffer[self.buffer_offset..self.buffer_offset + size];
            self.buffer_offset += size;
            Ok(Some(res))
        }
    }

    pub fn discard(&mut self, size: usize) -> io::Result<()> {
        let remaining = self.buffer_filled - self.buffer_offset;
        if size <= remaining {
            self.buffer_offset += size;
            return Ok(());
        }

        self.buffer_offset = 0;
        self.buffer_filled = 0;

        let mut left = size - remaining;
        while left != 0 {
            let n = left.min(self.buffer.len());
            match self.source.read(&mut self.buffer[..n]) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    left -= n;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    pub fn empty(&self) -> bool {
        self.source_exhausted && self.buffer_offset == self.buffer_filled
    }
}

pub fn get_progress_bar(total: u64, show: bool) -> anyhow::Result<ProgressBar> {
    if !show {
        return Ok(ProgressBar::hidden());
    }

    Ok(ProgressBar::new(total).with_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}) ETA: {eta}"
        )?.progress_chars("=> ")
    ))
}
