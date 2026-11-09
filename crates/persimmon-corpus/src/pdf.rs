// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! PDF samples, written at test time with lopdf and never committed. Each
//! is deterministic: no dates, and the same `/ID` every time, so the same
//! id always gives the same bytes.

use std::path::{Path, PathBuf};

use lopdf::content::{Content, Operation};
use lopdf::{
    Dictionary, Document, EncryptionState, EncryptionVersion, Object, ObjectId, Permissions,
    SaveOptions, Stream, dictionary,
};

/// Every PDF sample: `(id, file name, page count)`. The count is 0 for
/// samples no reader should open.
pub const PDF_SAMPLES: [(&str, &str, u32); 12] = [
    ("pdf-text", "text.pdf", 3),
    ("pdf-mixed", "mixed.pdf", 3),
    ("pdf-objstm", "objstm.pdf", 3),
    ("pdf-scan", "scan.pdf", 1),
    ("pdf-links", "links.pdf", 3),
    ("pdf-many", "many.pdf", 120),
    ("pdf-big-page", "big-page.pdf", 1),
    ("pdf-owner", "owner.pdf", 3),
    ("pdf-locked", "locked.pdf", 3),
    ("pdf-no-pages", "no-pages.pdf", 0),
    ("pdf-garbage", "garbage.pdf", 0),
    ("pdf-truncated", "truncated.pdf", 0),
];

const A4: [i64; 4] = [0, 0, 595, 842];
const LETTER: [i64; 4] = [0, 0, 612, 792];

/// Write the sample `id` into `dir` and return its path. Panics on an
/// unknown id.
pub fn write_pdf_sample(dir: &Path, id: &str) -> PathBuf {
    let name = PDF_SAMPLES
        .iter()
        .find(|(i, _, _)| *i == id)
        .unwrap_or_else(|| panic!("unknown PDF sample {id}"))
        .1;
    let bytes = match id {
        "pdf-text" => save(&mut text(3)),
        "pdf-mixed" => save(&mut mixed()),
        "pdf-objstm" => {
            let mut doc = text(3);
            let mut out = Vec::new();
            doc.save_with_options(
                &mut out,
                SaveOptions::builder()
                    .use_object_streams(true)
                    .use_xref_streams(true)
                    .build(),
            )
            .expect("save objstm.pdf");
            out
        }
        "pdf-scan" => save(&mut scan()),
        "pdf-links" => save(&mut links()),
        "pdf-many" => save(&mut text(120)),
        "pdf-big-page" => {
            let mut b = Builder::new();
            b.page(
                "Big",
                dictionary! { "MediaBox" => box_of([0, 0, 14400, 14400]) },
            );
            save(&mut b.finish(Dictionary::new(), Dictionary::new()))
        }
        "pdf-owner" => encrypted(""),
        "pdf-locked" => encrypted("secret"),
        "pdf-no-pages" => {
            let b = Builder::new();
            save(&mut b.finish(Dictionary::new(), Dictionary::new()))
        }
        "pdf-garbage" => {
            let mut v = b"%PDF-1.7\n".to_vec();
            v.extend(std::iter::repeat_n(b'Z', 2048));
            v
        }
        "pdf-truncated" => {
            let full = save(&mut text(120));
            full[..full.len() * 6 / 10].to_vec()
        }
        _ => unreachable!("every id in PDF_SAMPLES has a generator"),
    };
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    path
}

fn box_of(b: [i64; 4]) -> Object {
    Object::Array(b.iter().map(|&n| n.into()).collect())
}

/// A document under construction: one Helvetica font, pages added in order.
struct Builder {
    doc: Document,
    pages: ObjectId,
    font: ObjectId,
    kids: Vec<ObjectId>,
}

impl Builder {
    fn new() -> Builder {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        Builder {
            doc,
            pages,
            font,
            kids: Vec::new(),
        }
    }

    fn resources(&self) -> Dictionary {
        dictionary! { "Font" => dictionary! { "F1" => self.font } }
    }

    /// A page showing `text` in 48 pt Helvetica, with `extra` entries.
    fn page(&mut self, text: &str, extra: Dictionary) -> ObjectId {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 48.into()]),
                Operation::new("Td", vec![100.into(), 600.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let stream = Stream::new(Dictionary::new(), content.encode().expect("encode content"));
        let contents = self.doc.add_object(stream);
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => self.pages,
            "Contents" => contents,
        };
        page.extend(&extra);
        let id = self.doc.add_object(page);
        self.kids.push(id);
        id
    }

    /// Write the Pages node (with `pages_extra`), the catalog (with
    /// `catalog_extra`) and the trailer.
    fn finish(mut self, pages_extra: Dictionary, catalog_extra: Dictionary) -> Document {
        let mut pages = dictionary! {
            "Type" => "Pages",
            "Kids" => self.kids.iter().map(|&k| Object::Reference(k)).collect::<Vec<_>>(),
            "Count" => self.kids.len() as i64,
        };
        pages.extend(&pages_extra);
        self.doc
            .objects
            .insert(self.pages, Object::Dictionary(pages));
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => self.pages };
        catalog.extend(&catalog_extra);
        let root = self.doc.add_object(catalog);
        self.doc.trailer.set("Root", root);
        let id = Object::string_literal("persimmon-sample");
        self.doc
            .trailer
            .set("ID", Object::Array(vec![id.clone(), id]));
        self.doc
    }
}

fn save(doc: &mut Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save sample");
    out
}

/// `n` A4 pages, `Page 1`…: the box and fonts are inherited from the
/// Pages node, as many real PDFs do.
fn text(n: u32) -> Document {
    let mut b = Builder::new();
    for i in 1..=n {
        b.page(&format!("Page {i}"), Dictionary::new());
    }
    let pages_extra = dictionary! {
        "MediaBox" => box_of(A4),
        "Resources" => b.resources(),
    };
    let mut doc = b.finish(pages_extra, Dictionary::new());
    let info = doc.add_object(dictionary! { "Title" => Object::string_literal("Text sample") });
    doc.trailer.set("Info", info);
    doc
}

fn mixed() -> Document {
    let mut b = Builder::new();
    b.page("Page 1", dictionary! { "MediaBox" => box_of(LETTER) });
    b.page(
        "Page 2",
        dictionary! { "MediaBox" => box_of(A4), "Rotate" => 90 },
    );
    b.page(
        "Page 3",
        dictionary! { "MediaBox" => box_of(A4), "CropBox" => box_of([0, 0, 420, 595]) },
    );
    let pages_extra = dictionary! { "Resources" => b.resources() };
    b.finish(pages_extra, Dictionary::new())
}

/// A "scanned" page: one A4 page that is nothing but a JPEG.
fn scan() -> Document {
    let jpeg: &[u8] = include_bytes!("../samples/photo.jpg");
    let mut b = Builder::new();
    let image = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 320,
            "Height" => 240,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg.to_vec(),
    );
    let image = b.doc.add_object(image);
    let content = b"q 595 0 0 842 0 0 cm /Im0 Do Q".to_vec();
    let contents = b.doc.add_object(Stream::new(Dictionary::new(), content));
    let page = b.doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => b.pages,
        "MediaBox" => box_of(A4),
        "Contents" => contents,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image } },
    });
    b.kids.push(page);
    b.finish(Dictionary::new(), Dictionary::new())
}

/// Links, an outline and a form field: everything that points at other
/// pages.
fn links() -> Document {
    let mut b = Builder::new();
    let pages: Vec<ObjectId> = (1..=3)
        .map(|i| b.page(&format!("Page {i}"), Dictionary::new()))
        .collect();
    let fit = |page: ObjectId| Object::Array(vec![page.into(), "Fit".into()]);
    let uri = b.doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => box_of([100, 500, 300, 520]),
        "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.org/") },
    });
    let goto = b.doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => box_of([100, 400, 300, 420]),
        "Dest" => fit(pages[2]),
    });
    let field = b.doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Tx",
        "T" => Object::string_literal("name"),
        "V" => Object::string_literal("hello"),
        "Rect" => box_of([100, 300, 300, 320]),
        "P" => pages[1],
    });
    for (page, annots) in [(pages[0], vec![uri, goto]), (pages[1], vec![field])] {
        let dict = b
            .doc
            .get_object_mut(page)
            .and_then(Object::as_dict_mut)
            .expect("page dictionary");
        dict.set(
            "Annots",
            annots
                .into_iter()
                .map(Object::Reference)
                .collect::<Vec<_>>(),
        );
    }
    let outlines = b.doc.new_object_id();
    let items: Vec<ObjectId> = (0..3).map(|_| b.doc.new_object_id()).collect();
    for (i, &item) in items.iter().enumerate() {
        let mut d = dictionary! {
            "Title" => Object::string_literal(format!("Page {}", i + 1)),
            "Parent" => outlines,
            "Dest" => fit(pages[i]),
        };
        if i > 0 {
            d.set("Prev", items[i - 1]);
        }
        if i + 1 < items.len() {
            d.set("Next", items[i + 1]);
        }
        b.doc.objects.insert(item, Object::Dictionary(d));
    }
    b.doc.objects.insert(
        outlines,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => items[0],
            "Last" => items[2],
            "Count" => 3,
        }),
    );
    let pages_extra = dictionary! {
        "MediaBox" => box_of(A4),
        "Resources" => b.resources(),
    };
    let catalog_extra = dictionary! {
        "Outlines" => outlines,
        "AcroForm" => dictionary! { "Fields" => vec![Object::Reference(field)] },
    };
    let mut doc = b.finish(pages_extra, catalog_extra);
    let info = doc.add_object(dictionary! { "Title" => Object::string_literal("Links sample") });
    doc.trailer.set("Info", info);
    doc
}

/// `text.pdf`, encrypted with RC4-128: `user` is the password needed to
/// open it (empty for none), `owner` restricts it to printing.
fn encrypted(user: &str) -> Vec<u8> {
    let mut doc = text(3);
    let version = EncryptionVersion::V2 {
        document: &doc,
        owner_password: "owner",
        user_password: user,
        key_length: 128,
        permissions: Permissions::PRINTABLE,
    };
    let state = EncryptionState::try_from(version).expect("encryption state");
    doc.encrypt(&state).expect("encrypt sample");
    // For revision 3 only the first 16 bytes of /U are checked; lopdf pads
    // the rest with random bytes. Zero them so the sample is deterministic.
    let encrypt = doc
        .trailer
        .get(b"Encrypt")
        .and_then(Object::as_reference)
        .expect("an /Encrypt reference");
    let dict = doc
        .get_object_mut(encrypt)
        .and_then(Object::as_dict_mut)
        .expect("the encryption dictionary");
    let mut u = dict
        .get(b"U")
        .and_then(Object::as_str)
        .expect("/U")
        .to_vec();
    u[16..].fill(0);
    dict.set("U", Object::String(u, lopdf::StringFormat::Literal));
    save(&mut doc)
}
