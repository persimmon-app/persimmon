// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The one path that writes PDFs: convert-to-PDF, merge and split (DD-36).
//! Every input adds its pages, in order, to one new document.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use lopdf::{Document, Object, ObjectId, SaveOptions, dictionary};

use super::assemble::{append_pages, load_pdf};
use super::images::{add_image_page, image_page};
use crate::backend::Done;
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask, Quality, select_pages};

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
    let mut info = None;
    let total = task.inputs.len();
    for (done, (input, &from)) in task.inputs.iter().zip(&task.from).enumerate() {
        if from == FormatId::Pdf {
            let source = load_pdf(input)?;
            let count = source.get_pages().len() as u32;
            // Split picks pages; merge takes them all.
            let pages =
                select_pages(options.pages.as_ref(), count).map_err(|e| e.with_file(input))?;
            let appended =
                append_pages(&mut doc, source, &pages).map_err(|e| e.with_file(input))?;
            kids.extend(appended.pages);
            info = appended.info;
        } else {
            let page = image_page(input, from, options)?;
            kids.push(add_image_page(&mut doc, &page, quality).map_err(|e| e.with_file(input))?);
        }
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
    // A document made from one PDF (a split) keeps its title and author.
    if let ([_], Some(info)) = (task.inputs.as_slice(), info) {
        doc.trailer.set("Info", info);
    }
    // Drop everything no kept page reaches: the rest of each source.
    doc.prune_objects();
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

    // Merging and splitting PDFs.

    fn pdf(dir: &Path, id: &str) -> PathBuf {
        persimmon_corpus::pdf::write_pdf_sample(dir, id)
    }

    fn split(dir: &Path, input: PathBuf, pages: &str) -> PathBuf {
        let options = JobOptions {
            pages: Some(crate::job::PageRanges::parse(pages).unwrap()),
            ..JobOptions::default()
        };
        make(dir, Operation::Split, vec![input], options).unwrap()
    }

    fn merge(dir: &Path, inputs: Vec<PathBuf>) -> PathBuf {
        make(dir, Operation::Merge, inputs, JobOptions::default()).unwrap()
    }

    fn text_of(doc: &Document, n: u32) -> String {
        doc.extract_text(&[n]).unwrap()
    }

    fn page_objects(doc: &Document) -> usize {
        doc.objects
            .values()
            .filter(|o| {
                o.as_dict()
                    .is_ok_and(|d| d.get(b"Type").and_then(Object::as_name).ok() == Some(b"Page"))
            })
            .count()
    }

    #[test]
    fn merge_two_pdfs_in_order() {
        let dir = scratch("merge-two");
        let out = merge(&dir, vec![pdf(&dir, "pdf-text"), pdf(&dir, "pdf-mixed")]);
        let doc = Document::load(&out).unwrap();
        assert_eq!(doc.get_pages().len(), 6);
        assert!(text_of(&doc, 1).contains("Page 1"));
        assert!(text_of(&doc, 3).contains("Page 3"));
        assert_eq!(media_box(&doc, 4), vec![0.0, 0.0, 612.0, 792.0]);
        assert_eq!(rotate(&doc, 5), 90);
        assert!(doc.trailer.get(b"Info").is_err(), "merge writes no info");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "vips")]
    #[test]
    fn merge_images_and_pdfs() {
        let dir = scratch("merge-mixed");
        let inputs = vec![
            image(&dir, "img-jpeg"),
            pdf(&dir, "pdf-text"),
            image(&dir, "img-alpha"),
        ];
        let doc = Document::load(merge(&dir, inputs)).unwrap();
        assert_eq!(doc.get_pages().len(), 5);
        assert_eq!(media_box(&doc, 1), vec![0.0, 0.0, 320.0, 240.0]);
        assert!(text_of(&doc, 2).contains("Page 1"));
        assert_eq!(media_box(&doc, 5), vec![0.0, 0.0, 400.0, 300.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_keeps_selected_pages() {
        let dir = scratch("split-keep");
        let doc = Document::load(split(&dir, pdf(&dir, "pdf-text"), "3,1")).unwrap();
        assert_eq!(doc.get_pages().len(), 2);
        assert!(text_of(&doc, 1).contains("Page 1"));
        assert!(text_of(&doc, 2).contains("Page 3"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_drops_unselected_pages() {
        let dir = scratch("split-drop");
        let out = split(&dir, pdf(&dir, "pdf-links"), "1");
        let doc = Document::load(&out).unwrap();
        assert_eq!(page_objects(&doc), 1);
        let catalog = doc.catalog().unwrap();
        assert!(catalog.get(b"Outlines").is_err());
        assert_eq!(super::super::pdf_info(&out).unwrap().pages, 1);
        let many = pdf(&dir, "pdf-many");
        let source = std::fs::metadata(&many).unwrap().len();
        let one = std::fs::metadata(split(&dir, many, "1")).unwrap().len();
        assert!(one * 10 < source, "{one} bytes of {source}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_keeps_annotations_and_info() {
        let dir = scratch("split-annots");
        let doc = Document::load(split(&dir, pdf(&dir, "pdf-links"), "1")).unwrap();
        let annots = page(&doc, 1).get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(annots.len(), 2);
        let info = doc
            .get_dictionary(doc.trailer.get(b"Info").unwrap().as_reference().unwrap())
            .unwrap();
        assert_eq!(
            info.get(b"Title").unwrap().as_str().unwrap(),
            b"Links sample"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inherited_attributes_survive() {
        let dir = scratch("inherit");
        let doc = Document::load(merge(&dir, vec![pdf(&dir, "pdf-text")])).unwrap();
        for n in 1..=3 {
            assert!(page(&doc, n).get(b"MediaBox").is_ok(), "page {n}");
            assert!(page(&doc, n).get(b"Resources").is_ok(), "page {n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn objstm_pdf_merges() {
        let dir = scratch("objstm");
        let doc = Document::load(merge(
            &dir,
            vec![pdf(&dir, "pdf-objstm"), pdf(&dir, "pdf-text")],
        ))
        .unwrap();
        assert_eq!(doc.get_pages().len(), 6);
        assert!(text_of(&doc, 2).contains("Page 2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn owner_pdf_splits_unencrypted() {
        let dir = scratch("owner");
        let out = split(&dir, pdf(&dir, "pdf-owner"), "2");
        let doc = Document::load(&out).unwrap();
        assert!(doc.trailer.get(b"Encrypt").is_err());
        assert!(text_of(&doc, 1).contains("Page 2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merged_output_renders() {
        let dir = scratch("render-merged");
        let out = merge(&dir, vec![pdf(&dir, "pdf-text"), pdf(&dir, "pdf-scan")]);
        let merged = dir.join("merged.pdf");
        std::fs::rename(out, &merged).unwrap();
        let job = Job {
            op: Operation::Convert,
            inputs: vec![merged],
            target: Some(FormatId::Png),
            output: OutputPolicy::default(),
            options: JobOptions::default(),
        };
        let plan = job.plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        let temp = dir.join("pages.part");
        super::super::render::render(
            &plan.tasks[0],
            &job.options,
            &temp,
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_between_inputs() {
        let dir = scratch("cancel");
        let job = Job {
            op: Operation::Merge,
            inputs: vec![pdf(&dir, "pdf-text"), pdf(&dir, "pdf-mixed")],
            target: Some(FormatId::Pdf),
            output: OutputPolicy::default(),
            options: JobOptions::default(),
        };
        let plan = job.plan(&Registry::builtin());
        let cancel = CancelToken::new();
        let token = cancel.clone();
        let mut calls = 0;
        let e = build(
            &plan.tasks[0],
            &job.options,
            &dir.join("out.part"),
            &cancel,
            &mut |_| {
                calls += 1;
                token.cancel();
            },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert_eq!(calls, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
