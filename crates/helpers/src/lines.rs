//! Bounded, cancellation-safe newline framing for local protocols.
use std::io;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};

/// Reads complete newline-terminated frames without losing a partial frame
/// when the caller cancels `next` in a `select!`. Empty EOF is `None`;
/// unfinished EOF and oversized frames are errors, never truncated lines.
pub struct BoundedLines<R> {
    reader: BufReader<R>,
    partial: Vec<u8>,
    limit: usize,
}
impl<R: AsyncRead + Unpin> BoundedLines<R> {
    /// Wraps a stream with a maximum frame length, including its newline.
    pub fn new(reader: R, limit: usize) -> Self {
        Self {
            reader: BufReader::new(reader),
            partial: Vec::new(),
            limit,
        }
    }
    /// Reads the next complete frame. State survives cancellation of this future.
    pub async fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            let bytes = self.reader.fill_buf().await?;
            if bytes.is_empty() {
                return if self.partial.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "unfinished protocol frame",
                    ))
                };
            }
            let end = bytes.iter().position(|b| *b == b'\n').map(|n| n + 1);
            let n = end.unwrap_or(bytes.len());
            if self.partial.len().saturating_add(n) > self.limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "protocol frame exceeds limit",
                ));
            }
            self.partial.extend_from_slice(&bytes[..n]);
            self.reader.consume(n);
            if end.is_some() {
                return Ok(Some(std::mem::take(&mut self.partial)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt as _;

    #[tokio::test]
    async fn cancellation_keeps_partial_bytes_and_limits_are_exact() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let mut lines = BoundedLines::new(reader, 4);
        writer.write_all(b"ab").await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), lines.next())
                .await
                .is_err()
        );
        writer.write_all(b"c\n").await.unwrap();
        assert_eq!(lines.next().await.unwrap().unwrap(), b"abc\n");
        writer.write_all(b"abcd\n").await.unwrap();
        assert_eq!(
            lines.next().await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[tokio::test]
    async fn eof_is_distinct_from_a_truncated_frame() {
        let mut lines = BoundedLines::new(&b"a\nb"[..], 10);
        assert_eq!(lines.next().await.unwrap().unwrap(), b"a\n");
        assert_eq!(
            lines.next().await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(
            BoundedLines::new(&b""[..], 1)
                .next()
                .await
                .unwrap()
                .is_none()
        );
    }
}
