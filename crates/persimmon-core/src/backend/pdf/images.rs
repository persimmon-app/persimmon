// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Images as PDF pages (DD-36).
//!
//! A JPEG is embedded as it is (DCT) when a PDF can show it unchanged:
//! baseline, extended or progressive, 8-bit, gray or RGB, and either
//! upright or rotated by a multiple of 90°, which becomes the page's
//! `/Rotate`. Its EXIF, XMP, GPS and comment segments are removed first.
//! Every other image decodes through libvips and is stored losslessly
//! (Flate) when the source was lossless, else as JPEG. Pages are sized from
//! the image's recorded resolution, else at 72 dpi.

use std::io::Cursor;
use std::path::Path;

use image::codecs::jpeg::{JpegDecoder, JpegEncoder};
use image::metadata::Orientation;
use image::{ExtendedColorType, ImageDecoder, ImageEncoder};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

use crate::error::{Error, ErrorCode};
use crate::format::FormatId;
use crate::job::{JobOptions, Quality};

/// The resolution of an image that doesn't record a plausible one.
const DEFAULT_DPI: f32 = 72.0;

fn plausible(dpi: f32) -> Option<f32> {
    (50.0..=4800.0).contains(&dpi).then_some(dpi)
}

/// What a JPEG's headers say, read without decoding it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct JpegInfo {
    /// The SOF marker's number: 0 baseline, 1 extended, 2 progressive.
    pub sof: u8,
    pub precision: u8,
    pub width: u16,
    pub height: u16,
    pub components: u8,
    /// From the JFIF header, when it's plausible.
    pub dpi: Option<f32>,
    /// Whether an end-of-image marker follows the scan: a truncated file
    /// has none.
    pub has_eoi: bool,
}

/// One marker segment before the scan: the marker, where it starts (at its
/// `FF`), where its data starts, and where it ends.
struct Segment {
    marker: u8,
    start: usize,
    data: usize,
    end: usize,
}

/// The segments from SOI up to the first SOS, and where that SOS starts.
fn segments(bytes: &[u8]) -> Option<(Vec<Segment>, usize)> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 2;
    loop {
        if *bytes.get(i)? != 0xFF {
            return None;
        }
        let start = i;
        while *bytes.get(i + 1)? == 0xFF {
            i += 1; // fill bytes
        }
        let marker = bytes[i + 1];
        i += 2;
        if marker == 0xDA {
            return Some((out, start));
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            // No length: standalone markers.
            out.push(Segment {
                marker,
                start,
                data: i,
                end: i,
            });
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?]));
        if len < 2 || i + len > bytes.len() {
            return None;
        }
        out.push(Segment {
            marker,
            start,
            data: i + 2,
            end: i + len,
        });
        i += len;
    }
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

/// Read a JPEG's frame header, JFIF density and whether it ends properly.
/// `None` when it isn't a JPEG this can walk.
pub(crate) fn jpeg_info(bytes: &[u8]) -> Option<JpegInfo> {
    let (segs, sos) = segments(bytes)?;
    let mut info: Option<JpegInfo> = None;
    let mut dpi = None;
    for seg in &segs {
        let data = &bytes[seg.data..seg.end];
        match seg.marker {
            0xE0 if data.starts_with(b"JFIF\0") && data.len() >= 12 => {
                let x = f32::from(be16(&data[8..10]));
                dpi = match data[7] {
                    1 => plausible(x),
                    2 => plausible((x * 2.54).round()),
                    _ => None,
                };
            }
            m @ 0xC0..=0xCF if !matches!(m, 0xC4 | 0xC8 | 0xCC) && data.len() >= 6 => {
                info = Some(JpegInfo {
                    sof: m - 0xC0,
                    precision: data[0],
                    height: be16(&data[1..3]),
                    width: be16(&data[3..5]),
                    components: data[5],
                    dpi: None,
                    has_eoi: false,
                });
            }
            _ => {}
        }
    }
    let mut info = info.filter(|i| i.width > 0 && i.height > 0)?;
    info.dpi = dpi;
    let scan = sos + 2 + usize::from(be16(bytes.get(sos + 2..sos + 4)?));
    info.has_eoi = bytes
        .get(scan..)
        .is_some_and(|rest| rest.windows(2).any(|w| w == [0xFF, 0xD9]));
    Some(info)
}

/// The JPEG without its APP1–APP13, APP15 and COM segments: EXIF, XMP,
/// GPS, maker notes and comments. JFIF, ICC (APP2) and Adobe (APP14) stay,
/// since they affect how the image looks.
pub(crate) fn strip_jpeg_metadata(bytes: &[u8]) -> Vec<u8> {
    let Some((segs, sos)) = segments(bytes) else {
        return bytes.to_vec();
    };
    let mut out = bytes[..2].to_vec();
    for seg in segs
        .iter()
        .filter(|s| !matches!(s.marker, 0xE1..=0xED | 0xEF | 0xFE))
    {
        out.extend_from_slice(&bytes[seg.start..seg.end]);
    }
    out.extend_from_slice(&bytes[sos..]);
    out
}

/// An image's samples, ready for a PDF image object.
pub(crate) enum ImageData {
    /// A JPEG embedded as it is.
    Jpeg {
        bytes: Vec<u8>,
        components: u8,
        icc: Option<Vec<u8>>,
    },
    /// Decoded 8-bit samples: 1 (gray) or 3 (RGB) components, with alpha
    /// separate. `lossy` sources are stored as JPEG, the rest with Flate.
    /// Only libvips decodes, so a build without it never makes these.
    #[cfg_attr(not(feature = "vips"), allow(dead_code))]
    Pixels {
        components: u8,
        pixels: Vec<u8>,
        alpha: Option<Vec<u8>>,
        lossy: bool,
    },
}

/// One image, as one PDF page.
pub(crate) struct ImagePage {
    pub width: u32,
    pub height: u32,
    pub dpi: f32,
    /// The page's `/Rotate`: 0, 90, 180 or 270.
    pub rotate: u16,
    pub data: ImageData,
}

fn corrupt(input: &Path, reason: impl Into<String>) -> Error {
    Error::new(ErrorCode::CorruptInput, reason).with_file(input)
}

/// Read `input` as a PDF page: a JPEG as it is when it can be, else
/// decoded through libvips.
pub(crate) fn image_page(
    input: &Path,
    from: FormatId,
    options: &JobOptions,
) -> Result<ImagePage, Error> {
    if from == FormatId::Jpeg
        && let Some(page) = jpeg_as_is(input, options)?
    {
        return Ok(page);
    }
    decoded_page(input, from, options)
}

/// The JPEG at `input` as an embeddable page, or `None` when it has to be
/// decoded instead. A JPEG with no end marker is an error either way.
fn jpeg_as_is(input: &Path, options: &JobOptions) -> Result<Option<ImagePage>, Error> {
    let bytes =
        std::fs::read(input).map_err(|e| corrupt(input, format!("can't read this image ({e})")))?;
    let Some(info) = jpeg_info(&bytes) else {
        return Ok(None);
    };
    if !info.has_eoi {
        return Err(corrupt(input, "the image is truncated"));
    }
    let Ok(mut decoder) = JpegDecoder::new(Cursor::new(&bytes)) else {
        return Ok(None);
    };
    let rotate = match decoder.orientation().unwrap_or(Orientation::NoTransforms) {
        Orientation::NoTransforms => 0,
        Orientation::Rotate90 => 90,
        Orientation::Rotate180 => 180,
        Orientation::Rotate270 => 270,
        // Mirrored: a page can't flip, so the pixels must.
        _ => return Ok(None),
    };
    let icc = decoder.icc_profile().ok().flatten();
    let fits = options
        .max_size
        .is_none_or(|max| u32::from(info.width.max(info.height)) <= max);
    let embeddable = matches!(info.sof, 0..=2)
        && info.precision == 8
        && matches!(info.components, 1 | 3)
        && fits;
    if !embeddable {
        return Ok(None);
    }
    Ok(Some(ImagePage {
        width: u32::from(info.width),
        height: u32::from(info.height),
        dpi: info.dpi.unwrap_or(DEFAULT_DPI),
        rotate,
        data: ImageData::Jpeg {
            bytes: strip_jpeg_metadata(&bytes),
            components: info.components,
            icc,
        },
    }))
}

#[cfg(feature = "vips")]
fn decoded_page(input: &Path, from: FormatId, options: &JobOptions) -> Result<ImagePage, Error> {
    let d = crate::backend::image::decode_srgb8(input, from, options.max_size)?;
    let (components, pixels, alpha) = match d.bands {
        2 | 4 => {
            let n = usize::from(d.bands);
            let mut colour = Vec::with_capacity(d.pixels.len() / n * (n - 1));
            let mut alpha = Vec::with_capacity(d.pixels.len() / n);
            for px in d.pixels.chunks_exact(n) {
                colour.extend_from_slice(&px[..n - 1]);
                alpha.push(px[n - 1]);
            }
            (d.bands - 1, colour, Some(alpha))
        }
        b => (b, d.pixels, None),
    };
    Ok(ImagePage {
        width: d.width,
        height: d.height,
        dpi: d.dpi.unwrap_or(DEFAULT_DPI),
        rotate: 0,
        data: ImageData::Pixels {
            components,
            pixels,
            alpha,
            lossy: matches!(
                from,
                FormatId::Jpeg | FormatId::Webp | FormatId::Heic | FormatId::Avif
            ),
        },
    })
}

#[cfg(not(feature = "vips"))]
fn decoded_page(input: &Path, _: FormatId, _: &JobOptions) -> Result<ImagePage, Error> {
    Err(
        Error::new(ErrorCode::BackendMissing, "this build has no image backend")
            .with_file(input)
            .with_fix("install a build of prsm made with libvips"),
    )
}

/// A number for a content stream: up to 4 decimals, no trailing zeros.
fn num(x: f32) -> String {
    let s = format!("{x:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn internal(e: impl std::fmt::Display) -> Error {
    Error::new(ErrorCode::Internal, format!("couldn't build the PDF ({e})"))
}

/// Add `page`'s image and a page showing it to `doc`, and return the
/// page's id. Its `Parent` is left for the caller.
pub(crate) fn add_image_page(
    doc: &mut Document,
    page: &ImagePage,
    quality: Quality,
) -> Result<ObjectId, Error> {
    let (w, h) = (page.width, page.height);
    let device = |n: u8| {
        Object::Name(if n == 1 {
            b"DeviceGray".to_vec()
        } else {
            b"DeviceRGB".to_vec()
        })
    };
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(w),
        "Height" => i64::from(h),
        "BitsPerComponent" => 8,
    };
    let image = match &page.data {
        ImageData::Jpeg {
            bytes,
            components,
            icc,
        } => {
            let colour_space = match icc {
                Some(profile) => {
                    let mut stream = Stream::new(
                        dictionary! {
                            "N" => i64::from(*components),
                            "Alternate" => device(*components),
                        },
                        profile.clone(),
                    );
                    stream.compress().map_err(internal)?;
                    let id = doc.add_object(stream);
                    Object::Array(vec!["ICCBased".into(), id.into()])
                }
                None => device(*components),
            };
            dict.set("ColorSpace", colour_space);
            dict.set("Filter", "DCTDecode");
            Stream::new(dict, bytes.clone()).with_compression(false)
        }
        ImageData::Pixels {
            components,
            pixels,
            alpha,
            lossy,
        } => {
            dict.set("ColorSpace", device(*components));
            if let Some(alpha) = alpha {
                let mut mask = Stream::new(
                    dictionary! {
                        "Type" => "XObject",
                        "Subtype" => "Image",
                        "Width" => i64::from(w),
                        "Height" => i64::from(h),
                        "ColorSpace" => "DeviceGray",
                        "BitsPerComponent" => 8,
                    },
                    alpha.clone(),
                );
                mask.compress().map_err(internal)?;
                dict.set("SMask", doc.add_object(mask));
            }
            if *lossy {
                let mut jpeg = Vec::new();
                let colour = if *components == 1 {
                    ExtendedColorType::L8
                } else {
                    ExtendedColorType::Rgb8
                };
                JpegEncoder::new_with_quality(&mut jpeg, quality.jpeg())
                    .write_image(pixels, w, h, colour)
                    .map_err(internal)?;
                dict.set("Filter", "DCTDecode");
                Stream::new(dict, jpeg).with_compression(false)
            } else {
                let mut stream = Stream::new(dict, pixels.clone());
                stream.compress().map_err(internal)?;
                stream
            }
        }
    };
    let image = doc.add_object(image);
    let (pw, ph) = (w as f32 * 72.0 / page.dpi, h as f32 * 72.0 / page.dpi);
    let content = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", num(pw), num(ph));
    let contents = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let mut dict = dictionary! {
        "Type" => "Page",
        "MediaBox" => vec![0.into(), 0.into(), Object::Real(pw), Object::Real(ph)],
        "Contents" => contents,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image } },
    };
    if page.rotate != 0 {
        dict.set("Rotate", i64::from(page.rotate));
    }
    Ok(doc.add_object(dict))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> &'static [u8] {
        persimmon_corpus::IMAGE_SAMPLES
            .iter()
            .find(|s| s.0 == id)
            .unwrap_or_else(|| panic!("no sample {id}"))
            .2
    }

    #[test]
    fn jpeg_info_reads_the_frame() {
        let photo = jpeg_info(sample("img-jpeg")).unwrap();
        assert_eq!((photo.width, photo.height), (320, 240));
        assert_eq!((photo.precision, photo.components), (8, 3));
        assert!(matches!(photo.sof, 0..=2), "{}", photo.sof);
        assert!(photo.has_eoi);
        assert_eq!(jpeg_info(sample("img-cmyk")).unwrap().components, 4);
        assert!(!jpeg_info(sample("img-truncated")).unwrap().has_eoi);
        assert!(jpeg_info(b"not a jpeg").is_none());
    }

    #[test]
    fn strip_jpeg_metadata_drops_exif() {
        let original = sample("img-orient-6");
        let stripped = strip_jpeg_metadata(original);
        assert!(original.windows(2).any(|w| w == [0xFF, 0xE1]));
        assert!(jpeg_info(&stripped).is_some());
        // No APP1 marker before the image data.
        let sos = stripped.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        assert!(!stripped[..sos].windows(2).any(|w| w == [0xFF, 0xE1]));
        assert!(!stripped.windows(6).any(|w| w == b"Exif\0\0"));
        let a = image::load_from_memory(original).unwrap();
        let b = image::load_from_memory(&stripped).unwrap();
        assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    }
}
