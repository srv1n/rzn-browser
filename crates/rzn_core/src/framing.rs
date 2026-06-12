use std::error::Error;
use std::fmt;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    Empty,
    TooLarge { len: usize, max: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Empty => write!(f, "frame is empty"),
            Self::TooLarge { len, max } => {
                write!(f, "frame length {len} exceeds limit {max}")
            }
        }
    }
}

impl Error for FrameError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Empty | Self::TooLarge { .. } => None,
        }
    }
}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(FrameError::Io(error)),
    }

    let len = u32::from_le_bytes(len_buf) as usize;
    validate_frame_len(len)?;

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

pub async fn read_required_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, FrameError> {
    read_frame(reader).await?.ok_or_else(|| {
        FrameError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "stream closed before frame",
        ))
    })
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), FrameError> {
    validate_frame_len(payload.len())?;
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

fn validate_frame_len(len: usize) -> Result<(), FrameError> {
    if len == 0 {
        return Err(FrameError::Empty);
    }
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge {
            len,
            max: MAX_FRAME_SIZE,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{read_frame, write_frame, FrameError, MAX_FRAME_SIZE};

    #[tokio::test]
    async fn just_under_limit_round_trips() {
        let payload = vec![b'x'; MAX_FRAME_SIZE - 1];
        let mut encoded = Vec::new();

        write_frame(&mut encoded, &payload)
            .await
            .expect("write under limit");
        let mut input = encoded.as_slice();
        let decoded = read_frame(&mut input)
            .await
            .expect("read frame")
            .expect("frame present");

        assert_eq!(decoded.len(), MAX_FRAME_SIZE - 1);
        assert_eq!(decoded.first(), Some(&b'x'));
        assert_eq!(decoded.last(), Some(&b'x'));
    }

    #[tokio::test]
    async fn over_limit_write_is_rejected() {
        let payload = vec![b'x'; MAX_FRAME_SIZE + 1];
        let mut encoded = Vec::new();

        let error = write_frame(&mut encoded, &payload)
            .await
            .expect_err("write above limit");

        assert!(matches!(
            error,
            FrameError::TooLarge {
                len,
                max: MAX_FRAME_SIZE
            } if len == MAX_FRAME_SIZE + 1
        ));
        assert!(encoded.is_empty());
    }

    #[tokio::test]
    async fn over_limit_read_is_rejected() {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&((MAX_FRAME_SIZE as u32) + 1).to_le_bytes());
        let mut input = encoded.as_slice();

        let error = read_frame(&mut input).await.expect_err("read above limit");

        assert!(matches!(
            error,
            FrameError::TooLarge {
                len,
                max: MAX_FRAME_SIZE
            } if len == MAX_FRAME_SIZE + 1
        ));
    }

    #[tokio::test]
    async fn empty_frame_is_rejected() {
        let header = 0u32.to_le_bytes();
        let mut input = header.as_slice();

        let error = read_frame(&mut input).await.expect_err("read empty");

        assert!(matches!(error, FrameError::Empty));
    }
}
