//! The schema lock (§4.17 of the board plan). Every file in
//! `golden/format/` is pinned by its length and digest for the schema the
//! writer writes, so a `WREN_BLESS=1` rewrite of a golden file fails here
//! until the table and `schema` change together: a change to the bytes
//! written for an existing model is `schema = 2`, never an edit of the
//! schema 1 table. The files are read from disk, so a file added without a
//! row, or a row whose file is gone, fails too.

use std::path::PathBuf;

use crate::{parse, write, SCHEMA};

/// FNV-1a, 64 bits. Unlike `DefaultHasher` it never changes with the Rust
/// release, so the table stays valid.
fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Schema 1, locked 2026-11-14: each file's name without `.wren-board`,
/// its length and its digest, in bytewise order of name. Never edited; see
/// the module comment.
const SCHEMA_1: &[(&str, usize, u64)] = &[
    ("blank.in", 7, 0xd3a1_2f7f_0537_aae8),
    ("blank.out", 11, 0x5684_8b94_79f2_7988),
    ("bom.in", 251, 0x189a_001e_7a07_d552),
    ("bom.out", 248, 0x5e96_5ae9_df5a_06ef),
    ("colour-case.in", 332, 0x2958_bec6_06e0_a995),
    ("colour-case.out", 332, 0x83da_fc79_adb1_3855),
    ("comments-only.in", 39, 0xf119_9a41_01fc_9d71),
    ("comments-only.out", 11, 0x5684_8b94_79f2_7988),
    ("comments.in", 242, 0x944a_b99b_38c8_0089),
    ("comments.out", 90, 0x56e1_0f7a_5a88_2b92),
    ("crlf.in", 276, 0xa7b2_4ae1_8180_43a1),
    ("crlf.out", 251, 0x6ea6_d555_0da1_5f83),
    ("defaults.in", 568, 0xdcc3_460c_dea3_cf76),
    ("defaults.out", 340, 0x1d25_1202_c1ad_c6c8),
    ("editor-connectors.out", 950, 0xd646_b51d_bab9_4eb8),
    ("editor-flowchart.out", 3352, 0xa0ab_938b_1f45_df60),
    ("editor-paste.out", 984, 0x7db0_09f9_ac23_16d6),
    ("editor-styles.out", 4872, 0x417f_a70a_184a_71a4),
    ("empty.in", 0, 0xcbf2_9ce4_8422_2325),
    ("empty.out", 11, 0x5684_8b94_79f2_7988),
    ("escapes.out", 712, 0x21a0_c6e2_84f9_3a72),
    ("fields.out", 722, 0xd829_33b9_eceb_c29d),
    ("future.out", 1203, 0x8b98_6ec9_df67_fbd3),
    ("password-reset.out", 2450, 0xbc76_63e6_baca_697c),
    ("quoted-keys.out", 328, 0x2966_79a1_ffa1_dee7),
    ("requires.out", 135, 0xab11_6045_a398_68b0),
    ("shapes-and-ends.out", 1610, 0x0ed6_29b7_6733_a16e),
    ("shuffled.in", 365, 0x586a_454a_7085_6d44),
    ("shuffled.out", 368, 0x02fd_3ec7_dcc2_3fbe),
    ("strings.in", 373, 0xcf71_8fbb_8cd8_8fab),
    ("strings.out", 347, 0xbb65_e4d5_1760_8569),
    ("subtables.in", 441, 0xe883_73ad_2597_b6b6),
    ("subtables.out", 404, 0xec28_95eb_26ee_0833),
    ("toml11.in", 286, 0x4e78_360b_8d61_42b4),
    ("toml11.out", 279, 0x1270_bad1_a596_b107),
    ("unknown.out", 755, 0xe53f_1394_c019_5b71),
    ("values.out", 731, 0x66a1_0b32_aa27_22aa),
];

/// The table for the schema the writer writes.
fn frozen() -> &'static [(&'static str, usize, u64)] {
    match SCHEMA {
        1 => SCHEMA_1,
        other => panic!("schema {other} has no frozen golden files; add its table"),
    }
}

/// Every board file in `golden/format/`, named without `.wren-board`, in
/// bytewise order of name.
fn golden_files() -> Vec<(String, Vec<u8>)> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden/format");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wren-board"))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let bytes = std::fs::read(&path).unwrap();
            (name, bytes)
        })
        .collect();
    files.sort();
    files
}

#[test]
fn every_golden_file_is_the_one_its_schema_froze() {
    let files = golden_files();
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    let table: Vec<&str> = frozen().iter().map(|&(name, ..)| name).collect();
    assert_eq!(
        names, table,
        "golden/format/ and the frozen table name different files"
    );
    for ((name, bytes), &(_, len, hash)) in files.iter().zip(frozen()) {
        assert_eq!(
            (bytes.len(), digest(bytes)),
            (len, hash),
            "{name} differs from the file schema {SCHEMA} froze: a change to the bytes \
             written for an existing model needs schema = {} (§4.17)",
            SCHEMA + 1
        );
    }
}

#[test]
fn every_frozen_output_writes_to_itself() {
    let outputs: Vec<_> = golden_files()
        .into_iter()
        .filter(|(name, _)| name.ends_with(".out"))
        .collect();
    assert!(outputs.len() > 20, "{} outputs", outputs.len());
    for (name, bytes) in outputs {
        let text = String::from_utf8(bytes).unwrap_or_else(|_| panic!("{name}: not UTF-8"));
        let board = parse(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(write(&board) == text, "{name}: write(parse(x)) != x");
    }
}

#[test]
fn the_digest_is_fnv_1a() {
    assert_eq!(digest(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(digest(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(digest(b"foobar"), 0x8594_4171_f739_67e8);
}
