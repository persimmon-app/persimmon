// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The one path that writes PDFs: convert-to-PDF, merge and split (DD-36).
//! Every input adds its pages, in order, to one new document.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use lopdf::{Document, Object, ObjectId, SaveOptions, dictionary};

use super::images::{add_image_page, image_page};
use crate::backend::Done;
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask, Quality};

/// Build the task's PDF at `temp`. Cancel is checked after each input.
pub(crate) fn build(
    task: &PlannedTask,
    options: &JobOptions,
    temp: &Path,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<Done, Error> {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let quality = options.quality.unwrap_or(Quality::High);
    let mut kids = Vec::new();
    let total = task.inputs.len();
    for (done, (input, &from)) in task.inputs.iter().zip(&task.from).enumerate() {
        if from == FormatId::Pdf {
            return Err(
                Error::new(ErrorCode::NotImplemented, "merging PDFs isn't built yet")
                    .with_file(input),
            );
        }
        let page = image_page(input, from, options)?;
        kids.push(add_image_page(&mut doc, &page, quality).map_err(|e| e.with_file(input))?);
        progress((done + 1) as f32 / total as f32);
        if cancel.is_cancelled() {
            let e = Error::new(ErrorCode::Cancelled, "cancelled");
            return Err(match task.inputs.as_slice() {
                [one] => e.with_file(one),
                _ => e,
            });
        }
    }
    finish(&mut doc, pages, &kids);
    save(&mut doc, temp)?;
    Ok(Done::Written)
}

/// Give `doc` its page tree, holding `kids` in order, and its catalog.
fn finish(doc: &mut Document, pages: ObjectId, kids: &[ObjectId]) {
    for &kid in kids {
        if let Ok(Object::Dictionary(page)) = doc.get_object_mut(kid) {
            page.set("Parent", pages);
        }
    }
    doc.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids.iter().map(|&k| Object::Reference(k)).collect::<Vec<_>>(),
            "Count" => kids.len() as i64,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
}

/// Write `doc` to `temp`, with object and cross-reference streams.
fn save(doc: &mut Document, temp: &Path) -> Result<(), Error> {
    let fail = |e: &dyn std::fmt::Display| {
        Error::new(
            ErrorCode::OutputNotWritable,
            format!("couldn't write the PDF ({e})"),
        )
        .with_file(temp)
        .with_fix("choose another folder with --out <dir>")
    };
    let mut out = BufWriter::new(File::create(temp).map_err(|e| fail(&e))?);
    let options = SaveOptions::builder()
        .use_object_streams(true)
        .use_xref_streams(true)
        .build();
    doc.save_with_options(&mut out, options)
        .map_err(|e| fail(&e))?;
    std::io::Write::flush(&mut out).map_err(|e| fail(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::exec::CancelToken;
    use crate::format::FormatId;
    use crate::job::{Job, JobOptions, PlannedTask};
    use crate::output::OutputPolicy;
    use crate::registry::{Operation, Registry};
    use lopdf::{Dictionary, Document, Object, Stream};
    use std::path::{Path, PathBuf};

    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-build-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Copy the image sample `id` into `dir`.
    pub(crate) fn image(dir: &Path, id: &str) -> PathBuf {
        let (_, name, bytes, _) = persimmon_corpus::IMAGE_SAMPLES
            .iter()
            .find(|s| s.0 == id)
            .unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// Plan `op` on `inputs` into a PDF and build it; the PDF's path.
    pub(crate) fn make(
        dir: &Path,
        op: Operation,
        inputs: Vec<PathBuf>,
        options: JobOptions,
    ) -> Result<PathBuf, Error> {
        let job = Job {
            op,
            inputs,
            target: Some(FormatId::Pdf),
            output: OutputPolicy::default(),
            options,
        };
        let mut plan = job.plan(&Registry::builtin());
        if let Some(e) = plan.errors.pop() {
            return Err(e);
        }
        let task: PlannedTask = plan.tasks.remove(0);
        let temp = dir.join(format!("out-{}.pdf", std::process::id()));
        let _ = std::fs::remove_file(&temp);
        build(&task, &job.options, &temp, &CancelToken::new(), &mut |_| {})?;
        Ok(temp)
    }

    pub(crate) fn convert(dir: &Path, input: PathBuf) -> Result<PathBuf, Error> {
        make(dir, Operation::Convert, vec![input], JobOptions::default())
    }

    pub(crate) fn page(doc: &Document, n: u32) -> &Dictionary {
        doc.get_dictionary(doc.get_pages()[&n]).unwrap()
    }

    /// The page's `Im0` image stream.
    fn xobject(doc: &Document, n: u32) -> &Stream {
        let res = page(doc, n).get(b"Resources").unwrap().as_dict().unwrap();
        let xo = res.get(b"XObject").unwrap().as_dict().unwrap();
        let id = xo.get(b"Im0").unwrap().as_reference().unwrap();
        doc.get_object(id).unwrap().as_stream().unwrap()
    }

    fn name<'a>(dict: &'a Dictionary, key: &[u8]) -> &'a [u8] {
        dict.get(key).unwrap().as_name().unwrap()
    }

    fn media_box(doc: &Document, n: u32) -> Vec<f32> {
        page(doc, n)
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o.as_float().unwrap())
            .collect()
    }

    fn rotate(doc: &Document, n: u32) -> i64 {
        page(doc, n)
            .get(b"Rotate")
            .and_then(Object::as_i64)
            .unwrap_or(0)
    }

    fn sample_bytes(id: &str) -> &'static [u8] {
        persimmon_corpus::IMAGE_SAMPLES
            .iter()
            .find(|s| s.0 == id)
            .unwrap()
            .2
    }

    #[test]
    fn photo_jpeg_is_embedded_as_is() {
        let dir = scratch("photo");
        let doc = Document::load(convert(&dir, image(&dir, "img-jpeg")).unwrap()).unwrap();
        let im = xobject(&doc, 1);
        assert_eq!(name(&im.dict, b"Filter"), b"DCTDecode");
        assert_eq!(
            im.content,
            super::super::images::strip_jpeg_metadata(sample_bytes("img-jpeg"))
        );
        assert_eq!(media_box(&doc, 1), vec![0.0, 0.0, 320.0, 240.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exif_rotation_survives_to_pdf() {
        let dir = scratch("orient");
        let pdf = convert(&dir, image(&dir, "img-orient-6")).unwrap();
        let doc = Document::load(&pdf).unwrap();
        assert_eq!(rotate(&doc, 1), 90);
        assert_eq!(name(&xobject(&doc, 1).dict, b"Filter"), b"DCTDecode");
        // Rendered back, the page is portrait: 300×200 turned upright.
        let info = super::super::pdf_info(&pdf).unwrap();
        assert!(info.first_page.1 > info.first_page.0, "{info:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jpeg_metadata_stays_out_of_pdf() {
        let dir = scratch("exif");
        let pdf = convert(&dir, image(&dir, "img-orient-6")).unwrap();
        let bytes = std::fs::read(pdf).unwrap();
        assert!(!bytes.windows(6).any(|w| w == b"Exif\0\0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn p3_jpeg_keeps_its_profile() {
        let dir = scratch("p3");
        let doc = Document::load(convert(&dir, image(&dir, "img-p3")).unwrap()).unwrap();
        let cs = xobject(&doc, 1)
            .dict
            .get(b"ColorSpace")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(cs[0].as_name().unwrap(), b"ICCBased");
        let icc = doc
            .get_object(cs[1].as_reference().unwrap())
            .unwrap()
            .as_stream()
            .unwrap();
        assert_eq!(icc.dict.get(b"N").unwrap().as_i64().unwrap(), 3);
        assert_eq!(name(&icc.dict, b"Alternate"), b"DeviceRGB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_jpeg_is_corrupt_input() {
        let dir = scratch("truncated");
        let e = convert(&dir, image(&dir, "img-truncated")).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.reason, "the image is truncated");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_images_keeps_order() {
        let dir = scratch("order");
        let inputs = ["img-jpeg", "img-orient-6", "img-p3"]
            .map(|id| image(&dir, id))
            .to_vec();
        let pdf = make(&dir, Operation::Merge, inputs, JobOptions::default()).unwrap();
        let doc = Document::load(pdf).unwrap();
        assert_eq!(doc.get_pages().len(), 3);
        assert_eq!(media_box(&doc, 1), vec![0.0, 0.0, 320.0, 240.0]);
        assert_eq!(media_box(&doc, 2), vec![0.0, 0.0, 300.0, 200.0]);
        assert_eq!(
            (rotate(&doc, 1), rotate(&doc, 2), rotate(&doc, 3)),
            (0, 90, 0)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_page_size_follows_dpi() {
        use image::codecs::jpeg::{JpegEncoder, PixelDensity};
        let dir = scratch("dpi");
        let p = dir.join("scan.jpg");
        let mut enc = JpegEncoder::new_with_quality(std::fs::File::create(&p).unwrap(), 50);
        enc.set_pixel_density(PixelDensity::dpi(300));
        let gray = vec![200u8; 2480 * 3508];
        enc.encode(&gray, 2480, 3508, image::ExtendedColorType::L8)
            .unwrap();
        let doc = Document::load(convert(&dir, p).unwrap()).unwrap();
        let mb = media_box(&doc, 1);
        assert!(
            (mb[2] - 595.2).abs() < 1.0 && (mb[3] - 841.9).abs() < 1.0,
            "{mb:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "vips")]
    mod vips {
        use super::*;

        #[test]
        fn cmyk_jpeg_is_decoded() {
            let dir = scratch("cmyk");
            let doc = Document::load(convert(&dir, image(&dir, "img-cmyk")).unwrap()).unwrap();
            let im = xobject(&doc, 1);
            assert_eq!(name(&im.dict, b"Filter"), b"DCTDecode");
            assert_eq!(name(&im.dict, b"ColorSpace"), b"DeviceRGB");
            assert_ne!(im.content, sample_bytes("img-cmyk"));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn alpha_png_gets_a_soft_mask() {
            let dir = scratch("alpha");
            let doc = Document::load(convert(&dir, image(&dir, "img-alpha")).unwrap()).unwrap();
            let im = xobject(&doc, 1);
            let mask = doc
                .get_object(im.dict.get(b"SMask").unwrap().as_reference().unwrap())
                .unwrap()
                .as_stream()
                .unwrap();
            assert_eq!(name(&mask.dict, b"ColorSpace"), b"DeviceGray");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn deep_png_is_8_bit() {
            let dir = scratch("deep");
            let doc = Document::load(convert(&dir, image(&dir, "img-deep-16")).unwrap()).unwrap();
            let im = xobject(&doc, 1);
            assert_eq!(
                im.dict.get(b"BitsPerComponent").unwrap().as_i64().unwrap(),
                8
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn heic_is_lossy_dct() {
            let dir = scratch("heic");
            let doc = Document::load(convert(&dir, image(&dir, "img-heic")).unwrap()).unwrap();
            assert_eq!(name(&xobject(&doc, 1).dict, b"Filter"), b"DCTDecode");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn png_is_lossless_flate() {
            let dir = scratch("flate");
            let doc = Document::load(convert(&dir, image(&dir, "img-alpha")).unwrap()).unwrap();
            let im = xobject(&doc, 1);
            assert_eq!(name(&im.dict, b"Filter"), b"FlateDecode");
            let rgba = image::load_from_memory(sample_bytes("img-alpha"))
                .unwrap()
                .into_rgba8();
            let rgb: Vec<u8> = rgba.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
            assert!(
                im.decompressed_content().unwrap() == rgb,
                "colour samples differ"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn max_size_shrinks_a_jpeg() {
            let dir = scratch("max");
            let options = JobOptions {
                max_size: Some(100),
                ..JobOptions::default()
            };
            let pdf = make(
                &dir,
                Operation::Convert,
                vec![image(&dir, "img-jpeg")],
                options,
            )
            .unwrap();
            let doc = Document::load(pdf).unwrap();
            let im = xobject(&doc, 1);
            let w = im.dict.get(b"Width").unwrap().as_i64().unwrap();
            let h = im.dict.get(b"Height").unwrap().as_i64().unwrap();
            assert_eq!(w.max(h), 100);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[cfg(not(feature = "vips"))]
    #[test]
    fn png_without_vips_is_backend_missing() {
        let dir = scratch("no-vips");
        let e = convert(&dir, image(&dir, "img-alpha")).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
