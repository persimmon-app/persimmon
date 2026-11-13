// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The JSON shapes the app relies on. Changing one breaks the app's
//! TypeScript types, so each is pinned here.

#![cfg(feature = "serde")]

use persimmon_core::error::{Error, ErrorCode, FixKind};
use persimmon_core::exec::Outcome;
use persimmon_core::format::FormatId;
use persimmon_core::job::{Job, JobOptions, PageRanges, Quality};
use persimmon_core::output::OutputPolicy;
use persimmon_core::registry::{Operation, Registry};
use persimmon_core::update::{Finish, Update};
use serde_json::json;

#[test]
fn error_codes_serialise_as_their_names() {
    for c in ErrorCode::ALL {
        assert_eq!(serde_json::to_value(c).unwrap(), json!(c.as_str()));
        assert_eq!(
            serde_json::from_value::<ErrorCode>(json!(c.as_str())).unwrap(),
            c
        );
    }
}

#[test]
fn format_ids_serialise_as_their_names_and_round_trip() {
    for f in Registry::builtin().formats() {
        let v = serde_json::to_value(f.id).unwrap();
        assert_eq!(v, json!(f.id.as_str()));
        assert_eq!(serde_json::from_value::<FormatId>(v).unwrap(), f.id);
    }
}

#[test]
fn error_with_kinded_fix_shape() {
    let e = Error::new(ErrorCode::Usage, "it has 3 pages")
        .with_file("/a.pdf")
        .with_fix_as(
            FixKind::DropOption {
                option: "pages".into(),
            },
            "drop --pages",
        );
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({
            "code": "USAGE", "file": "/a.pdf", "reason": "it has 3 pages",
            "fix": {"kind": "dropOption", "option": "pages", "text": "drop --pages"}
        })
    );
}

#[test]
fn error_without_file_or_fix_shape() {
    let e = Error::new(ErrorCode::Cancelled, "cancelled");
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({"code": "CANCELLED", "file": null, "reason": "cancelled", "fix": null})
    );
}

#[test]
fn job_round_trips() {
    let job = Job {
        op: Operation::Convert,
        inputs: vec!["/a.png".into()],
        target: Some(FormatId::Webp),
        output: OutputPolicy::default(),
        options: JobOptions {
            quality: Some(Quality::Medium),
            max_size: Some(2048),
            strip_metadata: true,
            pages: Some(PageRanges::parse("1-3,5").unwrap()),
            ..Default::default()
        },
    };
    let json = serde_json::to_value(&job).unwrap();
    assert_eq!(json["op"], "convert");
    assert_eq!(json["target"], "webp");
    assert_eq!(json["options"]["quality"], "medium");
    assert_eq!(json["options"]["maxSize"], 2048);
    assert_eq!(json["options"]["stripMetadata"], true);
    assert_eq!(json["options"]["pages"], "1-3,5");
    assert_eq!(serde_json::from_value::<Job>(json).unwrap(), job);
}

#[test]
fn bad_page_ranges_dont_deserialise() {
    let r = serde_json::from_str::<JobOptions>(r#"{"pages":"3-1","stripMetadata":false}"#);
    assert!(r.is_err());
}

#[test]
fn job_options_never_deserialise_ffmpeg() {
    let o: JobOptions =
        serde_json::from_str(r#"{"ffmpeg":"/tmp/evil","stripMetadata":false}"#).unwrap();
    assert_eq!(o.ffmpeg, None);
    let with = JobOptions {
        ffmpeg: Some("/opt/ff/ffmpeg".into()),
        ..Default::default()
    };
    assert!(serde_json::to_value(&with).unwrap().get("ffmpeg").is_none());
}

#[test]
fn update_shapes() {
    let u = Update::Progress {
        index: 0,
        total: 2,
        fraction: 0.5,
    };
    assert_eq!(
        serde_json::to_value(&u).unwrap(),
        json!({"type": "progress", "index": 0, "total": 2, "fraction": 0.5})
    );
    let s = Update::Started { index: 0, total: 2 };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        json!({"type": "started", "index": 0, "total": 2})
    );
    let f = Update::Finished {
        index: 1,
        total: 2,
        result: Finish::Done(Outcome::Written("/o/a.webp".into())),
    };
    assert_eq!(
        serde_json::to_value(&f).unwrap()["result"],
        json!({"done": {"written": "/o/a.webp"}})
    );
    let n = Update::Finished {
        index: 1,
        total: 2,
        result: Finish::Done(Outcome::NotSmaller),
    };
    assert_eq!(
        serde_json::to_value(&n).unwrap()["result"],
        json!({"done": "notSmaller"})
    );
    let e = Update::Finished {
        index: 1,
        total: 2,
        result: Finish::Failed(Error::new(ErrorCode::Cancelled, "cancelled")),
    };
    assert_eq!(
        serde_json::to_value(&e).unwrap()["result"]["failed"]["code"],
        "CANCELLED"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_path_serialises_lossily() {
    use std::os::unix::ffi::OsStrExt;
    let p = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/a\xff.png"));
    let e = Error::new(ErrorCode::CorruptInput, "x").with_file(p.clone());
    assert_eq!(
        serde_json::to_value(&e).unwrap()["file"],
        "/tmp/a\u{fffd}.png"
    );
    let u = Update::Finished {
        index: 0,
        total: 1,
        result: Finish::Done(Outcome::Written(p)),
    };
    assert_eq!(
        serde_json::to_value(&u).unwrap()["result"]["done"]["written"],
        "/tmp/a\u{fffd}.png"
    );
}

#[test]
fn output_policy_never_deserialises_overwrite() {
    // The app never overwrites (DD-42): a client can't turn it on.
    let p: OutputPolicy = serde_json::from_str(r#"{"dir":"/x","overwrite":true}"#).unwrap();
    assert!(!p.overwrite);
    assert_eq!(p.dir, Some("/x".into()));
}
