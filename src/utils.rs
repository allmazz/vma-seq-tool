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

#[cfg(test)]
mod tests {
    use super::SequentialReader;
    use std::collections::VecDeque;
    use std::io::{self, Read};

    struct ScriptedReader<'a> {
        source: &'a [u8],
        steps: VecDeque<io::Result<usize>>,
    }

    impl Read for ScriptedReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let size = self.steps.pop_front().unwrap_or(Ok(buf.len()))?.min(buf.len());
            self.source.read(&mut buf[..size])
        }
    }

    #[test]
    fn reads_sequentially_across_buffer_boundaries() {
        let data: Vec<u8> = (0..31).collect();

        for capacity in 1..=8 {
            for size in 1..=capacity {
                let mut source = data.as_slice();
                let mut reader = SequentialReader::new(&mut source, capacity);

                for expected in data.chunks(size) {
                    assert_eq!(reader.read_exact(expected.len()).unwrap().unwrap(), expected);
                }

                assert!(reader.read_exact(1).unwrap().is_none());
                assert!(reader.empty());
            }
        }
    }

    #[test]
    fn empty_source_returns_none_and_marks_reader_empty() {
        let mut source = &b""[..];
        let mut reader = SequentialReader::new(&mut source, 4);

        assert!(!reader.empty());
        assert!(reader.read_exact(1).unwrap().is_none());
        assert!(reader.empty());
    }

    #[test]
    fn truncated_read_preserves_remaining_bytes() {
        let mut source = &b"abcde"[..];
        let mut reader = SequentialReader::new(&mut source, 4);

        assert_eq!(reader.read_exact(3).unwrap().unwrap(), b"abc");
        assert_eq!(reader.read_exact(3).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert!(!reader.empty());
        assert_eq!(reader.read_exact(3).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(reader.read_exact(2).unwrap().unwrap(), b"de");
        assert!(reader.empty());
    }

    #[test]
    fn oversized_read_returns_error_without_consuming_data() {
        let mut source = &b"abcdef"[..];
        let mut reader = SequentialReader::new(&mut source, 4);

        assert_eq!(reader.read_exact(1).unwrap().unwrap(), b"a");
        let error = reader.read_exact(5).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(error.to_string().contains("--sequential-buffer-size"));
        assert_eq!(reader.read_exact(4).unwrap().unwrap(), b"bcde");
    }

    #[test]
    fn zero_length_operations_do_not_consume_data() {
        let mut source = &b"abcd"[..];
        let mut reader = SequentialReader::new(&mut source, 4);

        assert!(reader.read_exact(0).unwrap().unwrap().is_empty());
        reader.discard(0).unwrap();
        assert_eq!(reader.read_exact(1).unwrap().unwrap(), b"a");
        assert!(reader.read_exact(0).unwrap().unwrap().is_empty());
        reader.discard(0).unwrap();
        assert_eq!(reader.read_exact(3).unwrap().unwrap(), b"bcd");
    }

    #[test]
    fn zero_capacity_buffer_allows_only_zero_length_reads() {
        let mut source = &b"a"[..];
        let mut reader = SequentialReader::new(&mut source, 0);

        assert!(reader.read_exact(0).unwrap().unwrap().is_empty());
        reader.discard(0).unwrap();
        assert_eq!(reader.read_exact(1).unwrap_err().kind(), io::ErrorKind::Other);
        assert_eq!(source, b"a");
    }

    #[test]
    fn read_exact_handles_short_reads_and_interruptions() {
        let mut source = ScriptedReader {
            source: b"abcdef",
            steps: VecDeque::from([Ok(1), Err(io::ErrorKind::Interrupted.into()), Ok(2), Ok(1)]),
        };
        let mut reader = SequentialReader::new(&mut source, 4);

        assert_eq!(reader.read_exact(4).unwrap().unwrap(), b"abcd");
        assert_eq!(reader.read_exact(2).unwrap().unwrap(), b"ef");
        assert!(reader.read_exact(1).unwrap().is_none());
    }

    #[test]
    fn read_exact_propagates_errors_and_preserves_partial_data() {
        let mut source = ScriptedReader {
            source: b"abcdef",
            steps: VecDeque::from([Ok(2), Err(io::Error::other("read failed"))]),
        };
        let mut reader = SequentialReader::new(&mut source, 4);

        let error = reader.read_exact(4).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "read failed");
        assert_eq!(reader.read_exact(4).unwrap().unwrap(), b"abcd");
        assert_eq!(reader.read_exact(2).unwrap().unwrap(), b"ef");
    }

    #[test]
    fn discard_skips_exactly_the_requested_bytes() {
        let data: Vec<u8> = (0..20).collect();

        for consumed in [0, 1, 4] {
            for discarded in [0, 1, 3, 4, 5, 12] {
                let mut source = data.as_slice();
                let mut reader = SequentialReader::new(&mut source, 4);

                assert_eq!(reader.read_exact(consumed).unwrap().unwrap(), &data[..consumed]);
                reader.discard(discarded).unwrap();
                let offset = consumed + discarded;
                assert_eq!(reader.read_exact(4).unwrap().unwrap(), &data[offset..offset + 4]);
            }
        }
    }

    #[test]
    fn discard_handles_short_reads_and_interruptions() {
        let mut source = ScriptedReader {
            source: b"abcdefghij",
            steps: VecDeque::from([Ok(2), Err(io::ErrorKind::Interrupted.into()), Ok(1), Ok(2)]),
        };
        let mut reader = SequentialReader::new(&mut source, 4);

        reader.discard(7).unwrap();
        assert_eq!(reader.read_exact(3).unwrap().unwrap(), b"hij");
    }

    #[test]
    fn discard_propagates_source_errors() {
        let mut source = ScriptedReader {
            source: b"abcdef",
            steps: VecDeque::from([Ok(2), Err(io::Error::other("discard failed"))]),
        };
        let mut reader = SequentialReader::new(&mut source, 4);

        let error = reader.discard(5).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "discard failed");
    }

    #[test]
    fn discard_past_eof_returns_unexpected_eof() {
        for consumed in [0, 1] {
            let mut source = &b"abc"[..];
            let mut reader = SequentialReader::new(&mut source, 4);

            reader.read_exact(consumed).unwrap();
            assert_eq!(reader.discard(4).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        }
    }

    #[test]
    fn discard_remaining_bytes_after_truncated_read_marks_reader_empty() {
        let mut source = &b"abc"[..];
        let mut reader = SequentialReader::new(&mut source, 4);

        assert_eq!(reader.read_exact(4).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert!(!reader.empty());
        reader.discard(3).unwrap();
        assert!(reader.empty());
    }
}
