//! `cargo +nightly run --example make-seeds` writes valid inputs, made with
//! throwaway keys (never a real stick's), into `corpus/<target>/`.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use ourstick_core::report::{seal, ComputerKind, ReaderKey, ReaderReport, ReportState};
use ourstick_core::ticket::AddressHint;
use ourstick_core::{
    encrypt_blob, ChaChaCipher, Entry, Manifest, ManifestHint, NodeId, StickKey, StickSigningKey,
    Ticket,
};

fn write(target: &str, name: &str, bytes: &[u8]) {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(target);
    fs::create_dir_all(&folder).expect("corpus folder");
    fs::write(folder.join(name), bytes).expect("seed written");
}

fn main() {
    let signing = StickSigningKey::from_bytes(&[7; 32]);
    let stick_id = signing.stick_id();
    let stick_key = StickKey::from_bytes([9; 32]);

    let ticket = Ticket {
        stick_id,
        node_id: NodeId([5; 32]),
        manifest_hint: ManifestHint {
            seq: 1_700_000_000_000,
            hash: [4; 32],
        },
        address_hints: vec![
            AddressHint::Ipv4 {
                ip: [192, 168, 1, 10],
                port: 4433,
            },
            AddressHint::Ipv6 {
                ip: [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
                port: 4433,
            },
            AddressHint::Relay {
                url: "https://relay.example/".into(),
            },
        ],
        stick_key: StickKey::from_bytes([9; 32]),
    };
    let link = ticket.to_link().expect("link");
    write("ticket", "link", link.as_bytes());
    write(
        "ticket",
        "bare",
        link.rsplit('#').next().expect("fragment").as_bytes(),
    );
    write(
        "ticket",
        "in-a-sentence",
        format!("(see {link}).").as_bytes(),
    );

    let manifest = Manifest {
        partial: false,
        name: "Photos 2026".into(),
        entries: vec![
            Entry {
                path: "a.txt".into(),
                size: 3,
                mtime: 1_700_000_000,
                hash: [1; 32],
            },
            Entry {
                path: "folder/sub/b.jpg".into(),
                size: 1 << 20,
                mtime: 1_700_000_001,
                hash: [2; 32],
            },
        ],
    };
    write("manifest", "two", &manifest.to_bytes().expect("manifest"));
    write(
        "manifest",
        "empty",
        &Manifest::default().to_bytes().expect("manifest"),
    );

    let record = signing.sign_record(1_700_000_000_000, [4; 32]);
    write("record", "signed", &record.to_bytes());

    let reader = ReaderKey::from_bytes(&[3; 32]);
    let report = ReaderReport {
        counter: 12,
        seq: 1_700_000_000_000,
        state: ReportState::Downloading { percent: 40 },
        everything: false,
        kind: ComputerKind::Linux,
        name: Some("Anna's laptop".into()),
    };
    write(
        "report",
        "plain",
        &postcard::to_allocvec(&report).expect("report"),
    );
    write(
        "report",
        "sealed",
        &seal(&report, &reader, &stick_id, &stick_key).expect("sealed"),
    );

    let chacha = ChaChaCipher::new(&stick_key);
    for (name, size) in [("small", 100usize), ("two-chunks", 70_000)] {
        let plain: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        write("blob", &format!("{name}-plain"), &plain);
        let mut blob = Vec::new();
        encrypt_blob(&chacha, &mut Cursor::new(&plain), &mut blob).expect("blob");
        write("blob", &format!("{name}-sealed"), &blob);
    }

    let site = Path::new(env!("CARGO_MANIFEST_DIR")).join("../apps/website/site/version.json");
    write(
        "notice",
        "live",
        &fs::read(site).expect("the site's version.json"),
    );
    let test = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/ourstick-core/src/new_version/test-version.json");
    write("notice", "test", &fs::read(test).expect("the test notice"));

    for (name, text) in [
        ("plain", "Holiday photos"),
        ("greek", "Φωτογραφίες"),
        ("bidi", "abc\u{202e}gpj.exe"),
        ("escape", "a\u{1b}[31mred"),
        ("joiner", "👩\u{200d}💻"),
    ] {
        write("names", name, text.as_bytes());
    }
    for (name, text) in [
        ("plain", "Photos/a.jpg\nPhotos/b.jpg\nnotes.txt"),
        ("windows", "CON.txt\nOURSTI~1 /a\nTrip. /x\na:b\nnul"),
        ("cases", "A\na\nÉ\nE\u{301}\nFolder/x\nfolder"),
        (
            "ours",
            ".ourstick-partial/x\n.OurStick-received\ndesktop.ini\n._x",
        ),
    ] {
        write("saved_paths", name, text.as_bytes());
    }
    println!("seeds written");
}
