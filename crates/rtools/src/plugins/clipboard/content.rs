//! One system offer becomes one ordered snapshot, independent of the source files.

use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use image::{
    DynamicImage, ImageDecoder as _, ImageFormat, ImageReader, Limits, metadata::Orientation,
};
use sha2::{Digest, Sha256};
use url::Url;

use super::{Error, Result};

pub(super) const BYTE_LIMIT: usize = 512 * 1024 * 1024;

/// Native sources provide one representation per item, in advertised order.
pub(super) enum Input {
    Text(String),
    Image(Vec<u8>),
    #[cfg(target_os = "windows")]
    Dib(Vec<u8>),
    File(PathBuf, Option<FileImage>),
}

/// A file offer may retain the original through an open handle or an equivalent native image.
pub(super) enum FileImage {
    Handle(File),
    #[cfg(target_os = "macos")]
    Encoded(Vec<u8>),
}

impl Input {
    /// Decodes a native Windows bitmap on the storage worker, after releasing the system clipboard.
    #[cfg(target_os = "windows")]
    fn decode_dib(bytes: Vec<u8>) -> Result<Vec<u8>> {
        use image::{DynamicImage, ImageDecoder as _, codecs::bmp::BmpDecoder};
        let mut decoder = BmpDecoder::new_without_file_header(Cursor::new(bytes))?;
        let (width, height) = decoder.dimensions();
        if u64::from(width) * u64::from(height) > 64 * 1024 * 1024 {
            return Err(Error::Invalid);
        }
        let mut limits = Limits::default();
        limits.max_alloc = Some(256 << 20);
        decoder.set_limits(limits)?;
        let mut png = Vec::new();
        DynamicImage::from_decoder(decoder)?
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
        Ok(png)
    }

    /// Opens candidate images while the offer is current, leaving normal files and folders alone.
    pub(super) fn file(path: PathBuf) -> Self {
        let image = ImageFormat::from_path(&path).is_ok_and(|format| format.reading_enabled());
        let handle = (image && path.is_file())
            .then(|| File::open(&path).ok())
            .flatten();
        Self::File(path, handle.map(FileImage::Handle))
    }

    /// Bounds reads even when the source grows or supplies a streaming representation.
    pub(super) fn read(mut reader: impl Read, limit: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        (&mut reader)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
}

/// Discriminants are persisted in SQLite and used by its summary queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
pub(super) enum Kind {
    Text,
    Image,
    Path,
    Missing,
}

impl TryFrom<i64> for Kind {
    type Error = Error;

    /// Rejects unknown on-disk discriminants without interpreting their bytes.
    fn try_from(value: i64) -> Result<Self> {
        match value {
            0 => Ok(Self::Text),
            1 => Ok(Self::Image),
            2 => Ok(Self::Path),
            3 => Ok(Self::Missing),
            _ => Err(Error::Invalid),
        }
    }
}

/// The searchable caption is separate from the reversible native file URL.
pub(super) struct Member {
    pub kind: Kind,
    pub caption: String,
    pub uri: String,
    pub bytes: Vec<u8>,
}

/// Bounds raster allocations independently from the budget for archiving encoded bytes.
pub(super) fn decode_image(
    bytes: &[u8],
    format: ImageFormat,
) -> image::ImageResult<(DynamicImage, Orientation)> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(256 << 20);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder()?;
    // Reserve the output buffer before giving the decoder the remaining working memory.
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    Ok((DynamicImage::from_decoder(decoder)?, orientation))
}

impl Member {
    /// Validates the image on the worker and keeps its original encoding for copy and preview.
    fn image(&mut self, bytes: Vec<u8>) -> Result<()> {
        let format = image::guess_format(&bytes)?;
        match decode_image(&bytes, format) {
            Ok(_) => {}
            // The original can still be restored as a file when its raster will not fit in memory.
            Err(image::ImageError::Limits(_)) => {
                tracing::debug!("archiving clipboard image beyond the raster decoding budget")
            }
            Err(err) => return Err(err.into()),
        }
        if self.caption.is_empty() {
            self.caption = format!("image.{}", format.extensions_str()[0]);
        }
        self.bytes = bytes;
        self.kind = Kind::Image;
        Ok(())
    }

    /// Keeps an unreadable image's place so successful siblings remain recoverable.
    fn capture(input: Input, remaining: usize) -> Result<Self> {
        let mut part = Self {
            kind: Kind::Missing,
            caption: String::new(),
            uri: String::new(),
            bytes: Vec::new(),
        };
        let image = match input {
            Input::Text(text) => {
                part.kind = Kind::Text;
                part.caption = text;
                return Ok(part);
            }
            Input::Image(bytes) => Ok(bytes),
            #[cfg(target_os = "windows")]
            Input::Dib(bytes) => Input::decode_dib(bytes),
            Input::File(path, handle) => {
                part.uri = Url::from_file_path(&path)
                    .map_err(|_| Error::Invalid)?
                    .into();
                part.caption = path.to_string_lossy().into_owned();
                if !ImageFormat::from_path(&path).is_ok_and(|f| f.reading_enabled())
                    || (handle.is_none() && path.is_dir())
                {
                    part.kind = Kind::Path;
                    return Ok(part);
                }
                // A large file selection can temporarily exhaust descriptors during acquisition.
                match handle.or_else(|| {
                    path.is_file()
                        .then(|| File::open(&path).ok())
                        .flatten()
                        .map(FileImage::Handle)
                }) {
                    Some(FileImage::Handle(file)) => Input::read(file, remaining),
                    #[cfg(target_os = "macos")]
                    Some(FileImage::Encoded(bytes)) => Ok(bytes),
                    None => Err(Error::Missing),
                }
            }
        };
        match image.and_then(|bytes| {
            if bytes.len() > remaining {
                return Err(Error::Limit);
            }
            part.image(bytes)
        }) {
            Ok(()) => {}
            Err(Error::Limit) => return Err(Error::Limit),
            Err(_) => {
                tracing::warn!("could not save one clipboard image");
            }
        }
        Ok(part)
    }
}

/// The digest includes field lengths and positions to distinguish group boundaries.
pub(super) struct Group {
    pub parts: Vec<Member>,
    pub size: usize,
    pub digest: Vec<u8>,
}

impl TryFrom<Vec<Input>> for Group {
    type Error = Error;

    /// Snapshots an offer within the same logical budget used for retention.
    fn try_from(inputs: Vec<Input>) -> Result<Self> {
        let mut digest = Sha256::new();
        let mut parts = Vec::with_capacity(inputs.len());
        let mut size = 0;
        for input in inputs {
            let part = Member::capture(input, BYTE_LIMIT - size)?;
            digest.update([part.kind as u8]);
            for field in [part.caption.as_bytes(), part.uri.as_bytes(), &part.bytes] {
                digest.update((field.len() as u64).to_le_bytes());
                digest.update(field);
                size += field.len();
            }
            if size > BYTE_LIMIT {
                return Err(Error::Limit);
            }
            parts.push(part);
        }
        Ok(Self {
            parts,
            size,
            digest: digest.finalize().to_vec(),
        })
    }
}

/// Data ready to publish in one system clipboard replacement.
pub(super) enum Payload {
    Text(String),
    Image(Vec<u8>),
    Files(Vec<PathBuf>),
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl Payload {
    /// Supplies a widely supported raster format for native clipboard receivers.
    pub fn png(bytes: &[u8]) -> Result<Vec<u8>> {
        let format = image::guess_format(bytes)?;
        if format == ImageFormat::Png {
            return Ok(bytes.to_vec());
        }
        let mut png = Vec::new();
        let (mut raster, orientation) = decode_image(bytes, format)?;
        // The new PNG carries pixels only, so bake in the source's display orientation.
        raster.apply_orientation(orientation);
        raster.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
        Ok(png)
    }
}

#[cfg(test)]
mod tests {
    /// A portrait stored as landscape pixels must keep its visual orientation when published as PNG.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn copied_png_preserves_the_jpeg_orientation() {
        use super::*;
        use image::{ImageEncoder as _, codecs::jpeg::JpegEncoder};

        let mut jpeg = Vec::new();
        let mut encoder = JpegEncoder::new(&mut jpeg);
        // A little-endian TIFF IFD with one EXIF orientation tag (6 = clockwise quarter-turn).
        let exif = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        encoder.set_exif_metadata(exif.to_vec()).unwrap();
        encoder
            .encode_image(&DynamicImage::new_rgb8(60, 40))
            .unwrap();
        let png = Payload::png(&jpeg).unwrap();
        let result = image::load_from_memory(&png).unwrap();
        assert_eq!((result.width(), result.height()), (40, 60));
    }
}
