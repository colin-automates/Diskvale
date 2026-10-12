use super::{format::*, source::*, *};
use std::{
    io::{Cursor, Read},
    sync::atomic::Ordering,
};

#[cfg(windows)]
#[test]
fn volume_access_failure_explains_permissions_only_when_denied() {
    let denied = volume_access_error(std::io::Error::from_raw_os_error(5));
    assert!(denied.contains("Run as administrator"), "{denied}");
    assert!(denied.contains("require administrator access"), "{denied}");
    let missing = volume_access_error(std::io::Error::from_raw_os_error(2));
    assert!(missing.contains("volume access failed"), "{missing}");
    assert!(!missing.contains("Run as administrator"), "{missing}");
}

fn put16(b: &mut [u8], i: usize, v: u16) {
    b[i..i + 2].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn regression_missing_terminal_extent_is_rejected_by_bulk_parser() {
    let mut bytes = fixture();
    let raw = record(
        20,
        false,
        0,
        vec![
            filename(30, "short.bin", 1),
            nonresident("", 8192, 4096, 0, 0, 0, &[0x11, 1, 50, 0]),
        ],
    );
    bytes[20 * 1024..21 * 1024].copy_from_slice(&raw);
    assert!(
        read_index(
            Cursor::new(bytes),
            Path::new("C:\\"),
            Geometry {
                record_size: 1024,
                sector: 512,
                cluster: 4096,
                valid: 65536,
                total_clusters: None,
            },
            &Progress::default(),
            "fixture"
        )
        .is_err()
    );
}

#[test]
fn missing_terminal_extension_and_preallocated_tail_are_not_complete() {
    let mut bytes = fixture();
    bytes[20 * 1024..21 * 1024].copy_from_slice(&record(
        20,
        false,
        0,
        vec![
            filename(30, "preallocated-tail.bin", 1),
            nonresident("", 100, 8192, 0, 0, 0, &[0x11, 1, 50, 0]),
        ],
    ));
    assert!(
        scan(bytes)
            .unwrap_err()
            .contains("terminal NTFS stream coverage")
    );
    let mut bytes = fixture();
    bytes[26 * 1024..27 * 1024].fill(0);
    assert!(scan(bytes).unwrap_err().contains("attribute-list record"));
}
#[test]
fn bulk_data_extents_are_bounded_by_known_volume_geometry() {
    let mut geometry = geometry(fixture().len());
    geometry.total_clusters = Some(1);
    assert!(
        read_index(
            Cursor::new(fixture()),
            Path::new("C:\\"),
            geometry,
            &Progress::default(),
            "fixture"
        )
        .unwrap_err()
        .contains("exceeds the volume")
    );
    geometry.total_clusters = Some(1_000_000);
    assert!(
        read_index(
            Cursor::new(fixture()),
            Path::new("C:\\"),
            geometry,
            &Progress::default(),
            "fixture"
        )
        .is_ok()
    );
}

#[test]
fn regression_directory_and_root_streams_are_accounted() {
    let mut bytes = fixture();
    for (id, parent, name) in [(5, 5, "."), (30, 5, "folder")] {
        let raw = record(
            id,
            true,
            0,
            vec![
                filename(parent, name, 1),
                resident_attr(0x80, "resident", b"abc"),
                nonresident("named", 100, 4096, 0, 0, 0, &[0x11, 1, 51, 0]),
            ],
        );
        bytes[id as usize * 1024..(id as usize + 1) * 1024].copy_from_slice(&raw);
    }
    let scan = read_index(
        Cursor::new(bytes),
        Path::new("C:\\"),
        Geometry {
            record_size: 1024,
            sector: 512,
            cluster: 4096,
            valid: 65536,
            total_clusters: None,
        },
        &Progress::default(),
        "fixture",
    )
    .unwrap();
    let baseline = read_index(
        Cursor::new(fixture()),
        Path::new("C:\\"),
        Geometry {
            record_size: 1024,
            sector: 512,
            cluster: 4096,
            valid: 65536,
            total_clusters: None,
        },
        &Progress::default(),
        "fixture",
    )
    .unwrap();
    assert_eq!(scan.nodes[0].allocated, baseline.nodes[0].allocated + 8192);
    assert_eq!(scan.nodes.iter().filter(|n| n.synthetic).count(), 2);
    assert_eq!(
        scan.extensions(crate::model::Metric::Allocated)
            .iter()
            .map(|t| t.1)
            .sum::<u64>(),
        scan.nodes[0].allocated
    );
    let totals = crate::query::map_totals(
        &scan,
        crate::model::Metric::Allocated,
        &crate::query::Query::parse(""),
        None,
    );
    assert_eq!(totals.bytes[0], scan.nodes[0].allocated);
    assert_eq!(totals.files[0], scan.stats.files);
    for (id, node) in scan.nodes.iter().enumerate().filter(|(_, n)| n.synthetic) {
        assert_eq!(scan.path(id), scan.path(node.parent.unwrap()));
        assert!(crate::recycle::Target::from_scan(&scan, id).is_err());
        let tiles = crate::treemap::nested(
            &scan,
            node.parent.unwrap(),
            crate::model::Metric::Allocated,
            crate::treemap::Rect {
                x: 0.0,
                y: 0.0,
                w: 2000.0,
                h: 1200.0,
            },
        );
        assert_eq!(
            tiles
                .iter()
                .find(|t| t.id == id && t.kind == crate::treemap::Kind::File)
                .unwrap()
                .size,
            node.allocated
        );
    }
    let output = tempfile::tempdir().unwrap();
    let csv_path = output.path().join("directory-streams.csv");
    crate::output::csv(&scan, &csv_path, &Progress::default()).unwrap();
    let records: Vec<_> = csv::Reader::from_path(csv_path)
        .unwrap()
        .records()
        .map(Result::unwrap)
        .collect();
    assert_eq!(records.iter().filter(|r| &r[8] == "true").count(), 2);
    assert_eq!(
        records
            .iter()
            .filter(|r| &r[1] == "false")
            .map(|r| r[3].parse::<u64>().unwrap())
            .sum::<u64>(),
        scan.nodes[0].allocated
    );
    for row in records.iter().filter(|r| &r[8] == "true") {
        assert!(!row[0].ends_with("Directory streams"));
        assert_eq!(&row[6], "0");
    }
}

#[test]
fn listed_attribute_identity_and_nonresident_list_completeness_are_enforced() {
    for wrong in [
        resident_attr(0x10, "", &[0; 32]),
        resident_attr(0x80, "wrong-name", b"abc"),
    ] {
        let mut bytes = fixture();
        bytes[26 * 1024..27 * 1024].copy_from_slice(&record(26, false, reference(25), vec![wrong]));
        assert!(scan(bytes).unwrap_err().contains("listed NTFS attribute"));
    }
    let mut bytes = fixture();
    let mut list = nonresident("", 32, 4096, 0, 0, 0, &[0x11, 1, 51, 0]);
    put32(&mut list, 0, 0x20);
    bytes[25 * 1024..26 * 1024].copy_from_slice(&record(
        25,
        false,
        0,
        vec![filename(30, "list.bin", 1), list],
    ));
    assert!(
        scan(bytes)
            .unwrap_err()
            .contains("Unsupported nonresident attribute list")
    );
}

#[test]
fn virtual_coverage_accepts_preallocation_and_sparse_directory_streams() {
    let mut bytes = fixture();
    bytes[20 * 1024..21 * 1024].copy_from_slice(&record(
        20,
        false,
        0,
        vec![
            filename(30, "preallocated.bin", 1),
            nonresident("", 100, 8192, 0, 0, 1, &[0x11, 2, 50, 0]),
        ],
    ));
    bytes[5 * 1024..6 * 1024].copy_from_slice(&record(
        5,
        true,
        0,
        vec![
            filename(5, ".", 1),
            nonresident(
                "sparse-named",
                8192,
                4096,
                0x8000,
                0,
                1,
                &[0x11, 1, 50, 0x01, 1, 0],
            ),
        ],
    ));
    let scan = scan(bytes).unwrap();
    assert_eq!(scan.nodes[0].own_allocated, 4096);
    assert_eq!(scan.nodes[0].own_logical, 8192);
    assert_eq!(
        scan.nodes
            .iter()
            .find(|n| n.name == "preallocated.bin")
            .unwrap()
            .allocated,
        8192
    );
    let file = scan.nodes.iter().find(|n| n.name == "large.bin").unwrap();
    assert_eq!(file.logical, 736);
    assert_eq!(file.named_logical, 100);
    assert_eq!(
        scan.nodes
            .iter()
            .filter(|n| !n.directory)
            .map(|n| n.allocated)
            .sum::<u64>(),
        scan.nodes[0].allocated
    );
}
fn put32(b: &mut [u8], i: usize, v: u32) {
    b[i..i + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], i: usize, v: u64) {
    b[i..i + 8].copy_from_slice(&v.to_le_bytes());
}
fn reference(i: u64) -> u64 {
    (1 << 48) | i
}
fn resident_attr(typ: u32, name: &str, value: &[u8]) -> Vec<u8> {
    let name: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let off = (24 + name.len()).next_multiple_of(8);
    let len = (off + value.len()).next_multiple_of(8);
    let mut a = vec![0; len];
    put32(&mut a, 0, typ);
    put32(&mut a, 4, len as u32);
    a[9] = (name.len() / 2) as u8;
    put16(&mut a, 10, 24);
    put32(&mut a, 16, value.len() as u32);
    put16(&mut a, 20, off as u16);
    a[24..24 + name.len()].copy_from_slice(&name);
    a[off..off + value.len()].copy_from_slice(value);
    a
}
fn filename(parent: u64, name: &str, namespace: u8) -> Vec<u8> {
    let utf: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut value = vec![0; 66 + utf.len()];
    put64(&mut value, 0, reference(parent));
    value[64] = (utf.len() / 2) as u8;
    value[65] = namespace;
    value[66..].copy_from_slice(&utf);
    resident_attr(0x30, "", &value)
}
fn nonresident(
    name: &str,
    logical: u64,
    allocated: u64,
    flags: u16,
    vcn: u64,
    highest: u64,
    pairs: &[u8],
) -> Vec<u8> {
    let utf: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let header = if flags & 0x80ff != 0 { 72 } else { 64 };
    let off = (header + utf.len()).next_multiple_of(8);
    let len = (off + pairs.len()).next_multiple_of(8);
    let mut a = vec![0; len];
    put32(&mut a, 0, 0x80);
    put32(&mut a, 4, len as u32);
    a[8] = 1;
    a[9] = (utf.len() / 2) as u8;
    put16(&mut a, 10, header as u16);
    put16(&mut a, 12, flags);
    put64(&mut a, 16, vcn);
    put64(&mut a, 24, highest);
    put16(&mut a, 32, off as u16);
    put64(&mut a, 40, if vcn == 0 { (highest + 1) * 4096 } else { 0 });
    put64(&mut a, 48, logical);
    put64(&mut a, 56, logical);
    if flags & 0x80ff != 0 {
        put64(&mut a, 64, allocated);
    } else {
        put64(&mut a, 40, allocated);
    }
    a[header..header + utf.len()].copy_from_slice(&utf);
    a[off..off + pairs.len()].copy_from_slice(pairs);
    a
}
fn record(index: u64, directory: bool, base: u64, attrs: Vec<Vec<u8>>) -> Vec<u8> {
    let mut b = vec![0; 1024];
    b[..4].copy_from_slice(b"FILE");
    put16(&mut b, 4, 48);
    put16(&mut b, 6, 3);
    put16(&mut b, 16, 1);
    put16(&mut b, 20, 56);
    put16(&mut b, 22, if directory { 3 } else { 1 });
    put32(&mut b, 28, 1024);
    put64(&mut b, 32, base);
    put32(&mut b, 44, index as u32);
    let mut off = 56;
    for a in attrs {
        assert!(off + a.len() + 4 < 1022);
        b[off..off + a.len()].copy_from_slice(&a);
        off += a.len();
    }
    put32(&mut b, off, u32::MAX);
    put32(&mut b, 24, (off + 8) as u32);
    put16(&mut b, 48, 0xabcd);
    let tail1 = [b[510], b[511]];
    let tail2 = [b[1022], b[1023]];
    b[50..52].copy_from_slice(&tail1);
    b[52..54].copy_from_slice(&tail2);
    put16(&mut b, 510, 0xabcd);
    put16(&mut b, 1022, 0xabcd);
    b
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 64 * 1024];
    let mut insert = |i: usize, b: Vec<u8>| bytes[i * 1024..(i + 1) * 1024].copy_from_slice(&b);
    insert(
        0,
        record(
            0,
            false,
            0,
            vec![
                filename(5, "$MFT", 1),
                nonresident("", 65536, 65536, 0, 0, 15, &[0x11, 16, 10, 0]),
            ],
        ),
    );
    let mut version = vec![0; 12];
    version[8] = 3;
    version[9] = 1;
    insert(
        3,
        record(
            3,
            false,
            0,
            vec![
                filename(5, "$Volume", 1),
                resident_attr(0x70, "", &version),
                resident_attr(0x80, "", &[]),
            ],
        ),
    );
    insert(5, record(5, true, 0, vec![filename(5, ".", 1)]));
    insert(
        20,
        record(
            20,
            false,
            0,
            vec![
                filename(30, "東京.txt", 1),
                resident_attr(0x80, "", b"hello"),
            ],
        ),
    );
    insert(
        21,
        record(
            21,
            false,
            0,
            vec![
                filename(30, "large.bin", 1),
                filename(5, "shortcut.bin", 1),
                filename(30, "LARGE~1.BIN", 2),
                nonresident("", 736, 4096, 0, 0, 0, &[0x11, 1, 50, 0]),
                nonresident("extra", 100, 4096, 0, 0, 0, &[0x11, 1, 51, 0]),
            ],
        ),
    );
    insert(
        22,
        record(
            22,
            false,
            0,
            vec![
                filename(30, "resident.rs", 1),
                resident_attr(0x80, "", &vec![7; 736]),
            ],
        ),
    );
    insert(
        23,
        record(
            23,
            false,
            0,
            vec![
                filename(31, "sparse.dat", 1),
                nonresident(
                    "",
                    1048576,
                    8192,
                    0x8000,
                    0,
                    255,
                    &[0x11, 2, 52, 0x02, 254, 0, 0],
                ),
            ],
        ),
    );
    insert(
        24,
        record(
            24,
            false,
            0,
            vec![
                filename(31, "compressed.dat", 1),
                nonresident("", 20000, 4096, 1, 0, 15, &[0x11, 1, 54, 0x01, 15, 0]),
            ],
        ),
    );
    let mut list = vec![0; 32];
    put32(&mut list, 0, 0x80);
    put16(&mut list, 4, 32);
    put64(&mut list, 16, reference(26));
    insert(
        25,
        record(
            25,
            false,
            0,
            vec![
                filename(30, "extended.gz", 1),
                resident_attr(0x20, "", &list),
            ],
        ),
    );
    insert(
        26,
        record(
            26,
            false,
            reference(25),
            vec![nonresident("", 5000, 8192, 0, 0, 1, &[0x11, 2, 60, 0])],
        ),
    );
    insert(
        27,
        record(
            27,
            false,
            0,
            vec![
                filename(30, "DOSONLY.TXT", 2),
                resident_attr(0x80, "", b"abc"),
            ],
        ),
    );
    insert(
        28,
        record(
            28,
            true,
            0,
            vec![filename(5, "junction", 1), resident_attr(0xc0, "", &[])],
        ),
    );
    insert(
        30,
        record(
            30,
            true,
            0,
            vec![filename(31, "Artifacts", 1), filename(31, "ARTIFA~1", 2)],
        ),
    );
    insert(31, record(31, true, 0, vec![filename(5, "Projects", 1)]));
    bytes
}
fn geometry(len: usize) -> Geometry {
    Geometry {
        record_size: 1024,
        sector: 512,
        cluster: 4096,
        valid: len as u64,
        total_clusters: None,
    }
}
fn scan(bytes: Vec<u8>) -> Result<Scan, String> {
    let len = bytes.len();
    read_index(
        Cursor::new(bytes),
        Path::new("C:\\synthetic-fixture"),
        geometry(len),
        &Progress::default(),
        "Synthetic NTFS fixture",
    )
}

#[test]
fn signed_mapping_pairs_and_sparse_runs() {
    assert_eq!(
        runs(&[0x11, 3, 100, 0x11, 2, 0xf6, 0x01, 4, 0]).unwrap(),
        vec![
            Run {
                lcn: Some(100),
                clusters: 3
            },
            Run {
                lcn: Some(90),
                clusters: 2
            },
            Run {
                lcn: None,
                clusters: 4
            }
        ]
    );
    assert!(runs(&[0x91]).is_err());
    assert!(runs(&[0x11, 1, 0xff, 0]).is_err());
}
#[test]
fn rejects_torn_sector_and_only_allows_restored_kernel_records() {
    let mut b = record(5, true, 0, vec![]);
    fixup(&mut b, 512, false).unwrap();
    assert!(fixup(&mut b, 512, false).is_err());
    fixup(&mut b, 512, true).unwrap();
    b[510] ^= 1;
    assert!(fixup(&mut b, 512, true).is_err());
}
#[test]
fn ignores_deleted_records() {
    let mut b = record(0, false, 0, vec![]);
    put16(&mut b, 22, 0);
    assert!(parse(&b, 42, 4096, None).unwrap().is_none());
}
#[test]
fn rejects_invalid_attributes_and_header_overlap() {
    let mut b = record(0, false, 0, vec![resident_attr(0x80, "", b"x")]);
    fixup(&mut b, 512, false).unwrap();
    put32(&mut b, 60, 4000);
    assert!(attributes(&b).is_err());
    let mut a = resident_attr(0x80, "", b"x");
    put16(&mut a, 20, 8);
    assert!(resident(&a).is_err());
}
#[test]
fn bulk_tree_preserves_sizes_hard_links_unicode_and_extension_records() {
    let scan = scan(fixture()).unwrap();
    assert_eq!(
        (
            scan.stats.files,
            scan.stats.folders,
            scan.stats.shared_links
        ),
        (10, 3, 1)
    );
    assert_eq!(scan.nodes[0].logical, 1141328);
    assert_eq!(scan.nodes[0].allocated, 94208);
    let resident = scan
        .nodes
        .iter()
        .position(|n| n.name == "resident.rs")
        .unwrap();
    assert_eq!(
        (scan.nodes[resident].logical, scan.nodes[resident].allocated),
        (736, 0)
    );
    assert!(scan.nodes.iter().any(|n| n.name == "東京.txt"));
    assert!(!scan.nodes.iter().any(|n| n.name == "LARGE~1.BIN"));
    let extended = scan.nodes.iter().find(|n| n.name == "extended.gz").unwrap();
    assert_eq!((extended.logical, extended.allocated), (5000, 8192));
    assert_eq!(scan.stats.file_metadata_queries, 0);
    assert_eq!(scan.stats.mft_records, 64);
    if let Some(path) = std::env::var_os("CANOPY_EXPORT_MFT_FIXTURE") {
        std::fs::write(path, fixture()).unwrap();
    }
}
#[test]
fn saved_index_uses_volume_version_and_same_bulk_parser() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut file, &fixture()).unwrap();
    let scan = scan_file(Path::new("C:\\fixture"), file.path(), &Progress::default()).unwrap();
    assert_eq!(scan.nodes[0].allocated, 94208);
    assert_eq!(scan.stats.engine, "Saved NTFS MFT (read only)");
    assert_eq!(
        scan.stats.runtime.provenance(),
        crate::model::Provenance::Replay
    );
    assert!(!scan.deletion_eligible());
    assert!(crate::recycle::Target::from_scan(&scan, 1).is_err());
    let mut bad = fixture();
    let mut b = record(
        3,
        false,
        0,
        vec![resident_attr(
            0x70,
            "",
            &[0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0],
        )],
    );
    bad[3072..4096].copy_from_slice(&b);
    b.clear();
    std::fs::write(file.path(), bad).unwrap();
    assert!(
        scan_file(Path::new("C:\\fixture"), file.path(), &Progress::default())
            .unwrap_err()
            .contains("version")
    );
}
#[test]
fn read_crosses_fragmented_extent_and_record_boundaries() {
    let mut disk = Cursor::new(vec![0u8; 4096]);
    disk.get_mut()[512..1024].fill(1);
    disk.get_mut()[2048..2560].fill(2);
    let mut reader = MappedReader::new(
        &mut disk,
        vec![
            Run {
                lcn: Some(1),
                clusters: 1,
            },
            Run {
                lcn: Some(4),
                clusters: 1,
            },
        ],
        512,
        1024,
    )
    .unwrap();
    let mut output = vec![0; 1024];
    reader.read_exact(&mut output).unwrap();
    assert_eq!(&output[..512], &vec![1; 512]);
    assert_eq!(&output[512..], &vec![2; 512]);
}
#[test]
fn extended_mapping_is_contiguous_and_within_volume() {
    let a = Segment {
        vcn: 0,
        highest: 1,
        runs: vec![Run {
            lcn: Some(10),
            clusters: 2,
        }],
    };
    let b = Segment {
        vcn: 2,
        highest: 3,
        runs: vec![Run {
            lcn: Some(20),
            clusters: 2,
        }],
    };
    assert_eq!(assemble(vec![b, a], 4096, 16384, 30).unwrap().len(), 2);
    assert!(
        assemble(
            vec![Segment {
                vcn: 1,
                highest: 1,
                runs: vec![Run {
                    lcn: Some(1),
                    clusters: 1
                }]
            }],
            4096,
            4096,
            30
        )
        .is_err()
    );
    assert!(
        assemble(
            vec![Segment {
                vcn: 0,
                highest: 1,
                runs: vec![Run {
                    lcn: Some(29),
                    clusters: 2
                }]
            }],
            4096,
            8192,
            30
        )
        .is_err()
    );
}

#[test]
fn fragmented_mft_bootstrap_resolves_attribute_list_extension_once() {
    let mut list = vec![0; 64];
    for (off, vcn, refno) in [(0, 0, reference(0)), (32, 2, reference(42))] {
        put32(&mut list, off, 0x80);
        put16(&mut list, off + 4, 32);
        put64(&mut list, off + 8, vcn);
        put64(&mut list, off + 16, refno);
    }
    let mut first = record(
        0,
        false,
        0,
        vec![
            nonresident("", 16384, 16384, 0, 0, 1, &[0x11, 2, 10, 0]),
            resident_attr(0x20, "", &list),
        ],
    );
    fixup(&mut first, 512, false).unwrap();
    let extension = record(
        42,
        false,
        reference(0),
        vec![nonresident("", 0, 0, 0, 2, 3, &[0x11, 2, 20, 0])],
    );
    let mut queries = 0;
    let result = mapping_from_record(
        &mut Cursor::new(vec![]),
        &first,
        geometry(16384),
        100,
        &Progress::default(),
        |_, r| {
            assert_eq!(r, reference(42));
            queries += 1;
            Ok(extension.clone())
        },
    )
    .unwrap();
    assert_eq!(queries, 1);
    assert_eq!(
        result.runs,
        vec![
            Run {
                lcn: Some(10),
                clusters: 2
            },
            Run {
                lcn: Some(20),
                clusters: 2
            }
        ]
    );
    let mut reused = extension;
    put16(&mut reused, 16, 2);
    assert!(
        mapping_from_record(
            &mut Cursor::new(vec![]),
            &first,
            geometry(16384),
            100,
            &Progress::default(),
            |_, _| Ok(reused.clone())
        )
        .is_err()
    );
}

#[test]
fn nonresident_attribute_list_reads_whole_sectors_not_its_logical_tail() {
    let mut list = vec![0; 64];
    for (off, vcn, refno) in [(0, 0, reference(0)), (32, 2, reference(42))] {
        put32(&mut list, off, 0x80);
        put16(&mut list, off + 4, 32);
        put64(&mut list, off + 8, vcn);
        put64(&mut list, off + 16, refno);
    }
    let mut list_attr = nonresident("", 64, 4096, 0, 0, 0, &[0x11, 1, 5, 0]);
    put32(&mut list_attr, 0, 0x20);
    let mut first = record(
        0,
        false,
        0,
        vec![
            nonresident("", 16384, 16384, 0, 0, 1, &[0x11, 2, 10, 0]),
            list_attr,
        ],
    );
    fixup(&mut first, 512, false).unwrap();
    let extension = record(
        42,
        false,
        reference(0),
        vec![nonresident("", 0, 0, 0, 2, 3, &[0x11, 2, 20, 0])],
    );
    struct SectorDisk(Cursor<Vec<u8>>);
    impl Read for SectorDisk {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            assert!(b.len().is_multiple_of(512));
            assert!(self.0.position().is_multiple_of(512));
            self.0.read(b)
        }
    }
    impl std::io::Seek for SectorDisk {
        fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
            self.0.seek(p)
        }
    }
    let mut disk = vec![0; 6 * 4096];
    disk[5 * 4096..5 * 4096 + 64].copy_from_slice(&list);
    let result = mapping_from_record(
        &mut SectorDisk(Cursor::new(disk)),
        &first,
        geometry(16384),
        100,
        &Progress::default(),
        |_, _| Ok(extension.clone()),
    )
    .unwrap();
    assert_eq!(result.runs.len(), 2);
}

// A volume-backed fixture: record zero and its extension are in fragmented MFT
// storage, while the attribute-list bytes are outside the MFT stream.
fn external_list_volume() -> (Vec<u8>, Geometry) {
    let mut mft = fixture();
    let mut name = filename(5, "$MFT", 1);
    put16(&mut name, 14, 1);
    let mut data = nonresident("", 65536, 65536, 0, 0, 1, &[0x11, 2, 10, 0]);
    put16(&mut data, 14, 2);
    let mut list = vec![0; 96];
    for (off, typ, vcn, owner, instance) in [
        (0, 0x30, 0, 0, 1),
        (32, 0x80, 0, 0, 2),
        (64, 0x80, 2, 42, 4),
    ] {
        put32(&mut list, off, typ);
        put16(&mut list, off + 4, 32);
        put64(&mut list, off + 8, vcn);
        put64(&mut list, off + 16, reference(owner));
        put16(&mut list, off + 24, instance);
    }
    let mut list_attr = nonresident("", list.len() as u64, 4096, 0, 0, 0, &[0x11, 1, 5, 0]);
    put32(&mut list_attr, 0, 0x20);
    put16(&mut list_attr, 14, 3);
    mft[..1024].copy_from_slice(&record(0, false, 0, vec![list_attr, name, data]));
    let mut continuation = nonresident("", 0, 0, 0, 2, 15, &[0x11, 14, 20, 0]);
    put16(&mut continuation, 14, 4);
    mft[42 * 1024..43 * 1024].copy_from_slice(&record(42, false, reference(0), vec![continuation]));
    let mut volume = vec![0; 100 * 4096];
    volume[5 * 4096..5 * 4096 + list.len()].copy_from_slice(&list);
    volume[10 * 4096..12 * 4096].copy_from_slice(&mft[..8192]);
    volume[20 * 4096..34 * 4096].copy_from_slice(&mft[8192..]);
    let mut geometry = geometry(mft.len());
    geometry.total_clusters = Some(100);
    (volume, geometry)
}

fn fixture_record_offset(index: usize) -> usize {
    let logical = index * 1024;
    if logical < 8192 {
        10 * 4096 + logical
    } else {
        20 * 4096 + logical - 8192
    }
}

fn scan_external_list_volume(volume: Vec<u8>, geometry: Geometry) -> Result<Scan, String> {
    let mut disk = Cursor::new(volume);
    let progress = Progress::default();
    let mapping = bootstrap_fixture(&mut disk, geometry, &progress)?;
    scan_volume_fixture(&mut disk, mapping, geometry, &progress)
}

fn bootstrap_fixture<R: Read + std::io::Seek>(
    disk: &mut R,
    geometry: Geometry,
    progress: &Progress,
) -> Result<Bootstrap, String> {
    use std::io::SeekFrom;
    disk.seek(SeekFrom::Start(fixture_record_offset(0) as u64))
        .unwrap();
    let mut first = vec![0; geometry.record_size];
    disk.read_exact(&mut first).unwrap();
    fixup(&mut first, geometry.sector, false)?;
    let mapping = mapping_from_record(disk, &first, geometry, 100, progress, |disk, reference| {
        disk.seek(SeekFrom::Start(
            fixture_record_offset(slot(reference)) as u64
        ))
        .map_err(|e| e.to_string())?;
        let mut raw = vec![0; geometry.record_size];
        disk.read_exact(&mut raw).map_err(|e| e.to_string())?;
        Ok(raw)
    })?;
    Ok(mapping)
}

fn scan_volume_fixture<R: Read + std::io::Seek>(
    disk: &mut R,
    mapping: Bootstrap,
    geometry: Geometry,
    progress: &Progress,
) -> Result<Scan, String> {
    let reader = MappedReader::with_bootstrap(disk, mapping, geometry)?;
    read_index_resolved(
        reader,
        Path::new("C:\\synthetic-fixture"),
        geometry,
        progress,
        "Volume fixture",
    )
    .map_err(|error| error.to_string())
}

#[test]
fn volume_backed_nonresident_record_zero_list_completes_full_pipeline() {
    let (volume, geometry) = external_list_volume();
    let scan = scan_external_list_volume(volume, geometry).unwrap();
    assert_eq!(scan.stats.mft_records, 64);
    assert_eq!(
        (
            scan.stats.files,
            scan.stats.folders,
            scan.stats.shared_links
        ),
        (10, 3, 1)
    );
    assert_eq!(
        (scan.nodes[0].logical, scan.nodes[0].allocated),
        (1141328, 94208)
    );
    assert_eq!(
        scan.nodes.iter().filter(|node| node.name == "$MFT").count(),
        1
    );
    assert!(!scan.stats.cancelled);
}

fn list_entry(typ: u32, name: &str, vcn: u64, owner: u64, instance: u16) -> Vec<u8> {
    let utf: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut entry = vec![0; (26 + utf.len()).next_multiple_of(8)];
    let len = entry.len();
    put32(&mut entry, 0, typ);
    put16(&mut entry, 4, len as u16);
    entry[6] = (utf.len() / 2) as u8;
    entry[7] = 26;
    put64(&mut entry, 8, vcn);
    put64(&mut entry, 16, reference(owner));
    put16(&mut entry, 24, instance);
    entry[26..26 + utf.len()].copy_from_slice(&utf);
    entry
}

fn external_list_attr(logical: usize, pairs: &[u8], clusters: u64) -> Vec<u8> {
    let mut attr = nonresident(
        "",
        logical as u64,
        clusters * 4096,
        0,
        0,
        clusters - 1,
        pairs,
    );
    put32(&mut attr, 0, 0x20);
    put16(&mut attr, 14, 2);
    attr
}

fn put_volume_record(volume: &mut [u8], index: usize, raw: &[u8]) {
    let offset = fixture_record_offset(index);
    volume[offset..offset + 1024].copy_from_slice(raw);
}

// Two required attributes share one extension. The larger variation spans two
// nonadjacent clusters and ends mid-sector, with distinct named attributes.
fn add_external_file_list(volume: &mut [u8], fragmented: bool) -> usize {
    let mut main = nonresident("", 5000, 8192, 0, 0, 1, &[0x11, 2, 60, 0]);
    put16(&mut main, 14, 1);
    let mut list = list_entry(0x30, "", 0, 25, 1);
    list.extend(list_entry(0x80, "", 0, 26, 1));
    if fragmented {
        let owners = [26, 40, 41, 43, 44, 45];
        for (group, owner) in owners.into_iter().enumerate() {
            let mut attrs = if group == 0 {
                vec![main.clone()]
            } else {
                Vec::new()
            };
            for part in 0..20 {
                let name = format!("s{:03}", group * 20 + part);
                let instance = (part + 2) as u16;
                let mut attr = resident_attr(0x80, &name, &[]);
                put16(&mut attr, 14, instance);
                attrs.push(attr);
                list.extend(list_entry(0x80, &name, 0, owner, instance));
            }
            put_volume_record(
                volume,
                owner as usize,
                &record(owner, false, reference(25), attrs),
            );
        }
        assert!(list.len() > 4096 && list.len() < 8192 && !list.len().is_multiple_of(512));
        volume[6 * 4096..7 * 4096].copy_from_slice(&list[..4096]);
        volume[8 * 4096..8 * 4096 + list.len() - 4096].copy_from_slice(&list[4096..]);
    } else {
        let mut named = resident_attr(0x80, "extra", b"abc");
        put16(&mut named, 14, 2);
        put_volume_record(
            volume,
            26,
            &record(26, false, reference(25), vec![main, named]),
        );
        list.extend(list_entry(0x80, "extra", 0, 26, 2));
        volume[6 * 4096..6 * 4096 + list.len()].copy_from_slice(&list);
    }
    let attr = if fragmented {
        external_list_attr(list.len(), &[0x11, 1, 6, 0x11, 1, 2, 0], 2)
    } else {
        external_list_attr(list.len(), &[0x11, 1, 6, 0], 1)
    };
    let mut name = filename(30, "extended.gz", 1);
    put16(&mut name, 14, 1);
    put_volume_record(volume, 25, &record(25, false, 0, vec![attr, name]));
    list.len()
}

#[test]
fn ordinary_nonresident_lists_resolve_fragmented_storage_without_double_merging() {
    for fragmented in [false, true] {
        let (mut volume, geometry) = external_list_volume();
        add_external_file_list(&mut volume, fragmented);
        let scan = scan_external_list_volume(volume, geometry).unwrap();
        assert_eq!(
            (scan.nodes[0].logical, scan.nodes[0].allocated),
            (1141328, 94208)
        );
        assert_eq!((scan.stats.files, scan.stats.shared_links), (10, 1));
        let file = scan
            .nodes
            .iter()
            .find(|node| node.name == "extended.gz")
            .unwrap();
        assert_eq!((file.logical, file.allocated), (5000, 8192));
        assert_eq!(file.named_logical, if fragmented { 0 } else { 3 });
        assert_eq!(scan.stats.mft_records, 64);
    }
}

#[test]
fn directory_nonresident_list_uses_the_same_resolver_and_stream_bucket() {
    let (mut volume, geometry) = external_list_volume();
    let mut list = list_entry(0x30, "", 0, 30, 0);
    list.extend(list_entry(0x80, "named", 0, 45, 1));
    volume[7 * 4096..7 * 4096 + list.len()].copy_from_slice(&list);
    let mut data = nonresident("named", 100, 4096, 0, 0, 0, &[0x11, 1, 90, 0]);
    put16(&mut data, 14, 1);
    put_volume_record(
        &mut volume,
        45,
        &record(45, true, reference(30), vec![data]),
    );
    put_volume_record(
        &mut volume,
        30,
        &record(
            30,
            true,
            0,
            vec![
                external_list_attr(list.len(), &[0x11, 1, 7, 0], 1),
                filename(31, "Artifacts", 1),
            ],
        ),
    );
    let scan = scan_external_list_volume(volume, geometry).unwrap();
    assert_eq!(scan.nodes[0].allocated, 94208 + 4096);
    let folder = scan
        .nodes
        .iter()
        .find(|node| node.name == "Artifacts")
        .unwrap();
    assert_eq!((folder.own_logical, folder.own_allocated), (100, 4096));
    assert_eq!(scan.nodes.iter().filter(|node| node.synthetic).count(), 1);
}

#[test]
fn external_required_entries_reject_wrong_type_name_instance_vcn_or_reference() {
    for defect in [
        "type",
        "name",
        "instance",
        "vcn",
        "reference",
        "sequence",
        "owner",
        "missing",
    ] {
        let (mut volume, geometry) = external_list_volume();
        add_external_file_list(&mut volume, false);
        let list = 6 * 4096 + 32;
        let extension = fixture_record_offset(26);
        match defect {
            "type" => put32(&mut volume, list, 0x10),
            "name" => put16(&mut volume, list + 32 + 26, b'x' as u16),
            "instance" => put16(&mut volume, list + 24, 9),
            "vcn" => put64(&mut volume, list + 8, 1),
            "reference" => put64(&mut volume, list + 16, reference(27)),
            "sequence" => put16(&mut volume, extension + 16, 2),
            "owner" => put64(&mut volume, extension + 32, reference(27)),
            "missing" => volume[extension..extension + 1024].fill(0),
            _ => unreachable!(),
        }
        assert!(
            scan_external_list_volume(volume, geometry).is_err(),
            "accepted {defect}"
        );
    }
}

#[test]
fn bootstrap_metadata_is_rechecked_against_bulk_records_and_fresh_list_bytes() {
    for defect in [
        "base_sequence",
        "base_mapping",
        "extension_sequence",
        "extension_mapping",
        "extension_missing",
        "external_list",
    ] {
        let (volume, geometry) = external_list_volume();
        let progress = Progress::default();
        let mut disk = Cursor::new(volume);
        let mapping = bootstrap_fixture(&mut disk, geometry, &progress).unwrap();
        let volume = disk.get_mut();
        match defect {
            "base_sequence" => put16(volume, fixture_record_offset(0) + 16, 2),
            "base_mapping" => {
                let mut raw = record(
                    0,
                    false,
                    0,
                    vec![
                        filename(5, "$MFT", 1),
                        nonresident("", 65536, 65536, 0, 0, 15, &[0x11, 16, 11, 0]),
                    ],
                );
                // Same slot and sequence, but a different mapping/list layout.
                put_volume_record(volume, 0, &raw);
                raw.clear();
            }
            "extension_sequence" => put16(volume, fixture_record_offset(42) + 16, 2),
            "extension_mapping" => {
                let mut attr = nonresident("", 0, 0, 0, 2, 15, &[0x11, 14, 21, 0]);
                put16(&mut attr, 14, 4);
                put_volume_record(volume, 42, &record(42, false, reference(0), vec![attr]));
            }
            "extension_missing" => {
                let start = fixture_record_offset(42);
                volume[start..start + 1024].fill(0);
            }
            "external_list" => put16(volume, 5 * 4096 + 24, 9),
            _ => unreachable!(),
        }
        let error = scan_volume_fixture(&mut disk, mapping, geometry, &progress).unwrap_err();
        assert!(error.contains("bootstrap"), "{defect}: {error}");
    }
}

#[test]
fn external_list_lengths_mapping_bounds_and_truncation_fail_closed() {
    for defect in [
        "oversized",
        "short_initialized",
        "short_allocation",
        "out_of_volume",
        "missing_coverage",
        "sparse",
        "compressed",
        "empty_bytes",
        "truncated_entries",
        "truncated",
    ] {
        let (mut volume, geometry) = external_list_volume();
        let len = add_external_file_list(&mut volume, false);
        let mut attr = external_list_attr(len, &[0x11, 1, 6, 0], 1);
        match defect {
            "oversized" => put64(&mut attr, 48, 16 * 1024 * 1024 + 1),
            "short_initialized" => put64(&mut attr, 56, len as u64 - 1),
            "short_allocation" => put64(&mut attr, 40, 0),
            "out_of_volume" => attr = external_list_attr(len, &[0x11, 1, 100, 0], 1),
            "missing_coverage" => put64(&mut attr, 24, 1),
            "sparse" => attr = external_list_attr(len, &[0x01, 1, 0], 1),
            "compressed" => put16(&mut attr, 12, 1),
            "empty_bytes" => volume[6 * 4096..7 * 4096].fill(0),
            "truncated_entries" => put16(&mut volume, 6 * 4096 + 4, (len + 8) as u16),
            "truncated" => {
                attr = external_list_attr(len, &[0x11, 1, 80, 0], 1);
                volume[80 * 4096..80 * 4096 + len].copy_from_slice(
                    &list_entry(0x30, "", 0, 25, 1)
                        .into_iter()
                        .chain(list_entry(0x80, "", 0, 26, 1))
                        .chain(list_entry(0x80, "extra", 0, 26, 2))
                        .collect::<Vec<_>>(),
                );
                volume.truncate(80 * 4096 + 10);
            }
            _ => unreachable!(),
        }
        put_volume_record(
            &mut volume,
            25,
            &record(25, false, 0, vec![filename(30, "extended.gz", 1), attr]),
        );
        assert!(
            scan_external_list_volume(volume, geometry).is_err(),
            "accepted {defect}"
        );
    }
}

struct SectorVolume<'a> {
    disk: Cursor<Vec<u8>>,
    progress: &'a Progress,
    list_reads: usize,
    cancel_second_list_read: bool,
}
impl Read for SectorVolume<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        assert!(bytes.len().is_multiple_of(512), "unaligned read length");
        assert!(
            self.disk.position().is_multiple_of(512),
            "unaligned read offset"
        );
        if self.disk.position() == 5 * 4096 {
            self.list_reads += 1;
            if self.cancel_second_list_read && self.list_reads == 2 {
                self.progress.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.disk.read(bytes)
    }
}
impl std::io::Seek for SectorVolume<'_> {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.disk.seek(position)
    }
}

#[test]
fn fragmented_external_list_reads_are_sector_aligned_through_bulk_pipeline() {
    let (mut volume, geometry) = external_list_volume();
    let len = add_external_file_list(&mut volume, true);
    // Padding beyond logical EOF must never be interpreted as list entries.
    volume[8 * 4096 + len - 4096..9 * 4096].fill(0xff);
    let progress = Progress::default();
    let mut disk = SectorVolume {
        disk: Cursor::new(volume),
        progress: &progress,
        list_reads: 0,
        cancel_second_list_read: false,
    };
    let mapping = bootstrap_fixture(&mut disk, geometry, &progress).unwrap();
    let scan = scan_volume_fixture(&mut disk, mapping, geometry, &progress).unwrap();
    assert_eq!(scan.nodes[0].allocated, 94208);
    assert_eq!(disk.list_reads, 2); // bootstrap plus fresh bulk validation
}

#[test]
fn metadata_reads_restore_the_sequential_cursor_on_success_and_failure() {
    use super::source::IndexSource;
    let (volume, geometry) = external_list_volume();
    let progress = Progress::default();
    let mut disk = Cursor::new(volume);
    let expected = [
        disk.get_ref()[10 * 4096..12 * 4096].to_vec(),
        disk.get_ref()[20 * 4096..34 * 4096].to_vec(),
    ]
    .concat();
    let mut first =
        disk.get_ref()[fixture_record_offset(0)..fixture_record_offset(0) + 1024].to_vec();
    fixup(&mut first, 512, false).unwrap();
    let attrs = attributes(&first).unwrap();
    let list = attrs
        .iter()
        .find(|attr| u32_at(attr, 0) == Some(0x20))
        .unwrap();
    let mapping = vec![
        Run {
            lcn: Some(10),
            clusters: 2,
        },
        Run {
            lcn: Some(20),
            clusters: 14,
        },
    ];
    let mut reader = MappedReader::new(&mut disk, mapping, 4096, 65536).unwrap();
    let mut prefix = vec![0; 1024];
    reader.read_exact(&mut prefix).unwrap();
    reader.resolve_list(list, geometry, &progress).unwrap();
    let mut invalid = external_list_attr(32, &[0x11, 1, 7, 0], 1); // zero bytes
    assert!(reader.resolve_list(&invalid, geometry, &progress).is_err());
    put64(&mut invalid, 48, 0);
    assert!(reader.resolve_list(&invalid, geometry, &progress).is_err());
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    // Slot 3 and the later root/file records prove continuation of both runs.
    let mut flat = prefix;
    flat.extend(rest);
    assert_eq!(flat, expected);
    assert_eq!(&flat[3 * 1024..3 * 1024 + 4], b"FILE");
    let scan = read_index(
        Cursor::new(flat),
        Path::new("C:\\fixture"),
        geometry,
        &progress,
        "flat",
    );
    assert!(scan.unwrap_err().contains("external volume clusters"));
}

#[test]
fn cancellation_during_external_resolution_produces_partial_read_only_result() {
    let (volume, geometry) = external_list_volume();
    let progress = Progress::default();
    let mut disk = SectorVolume {
        disk: Cursor::new(volume),
        progress: &progress,
        list_reads: 0,
        cancel_second_list_read: true,
    };
    let mapping = bootstrap_fixture(&mut disk, geometry, &progress).unwrap();
    let scan = scan_volume_fixture(&mut disk, mapping, geometry, &progress).unwrap();
    assert!(scan.stats.cancelled);
    assert_eq!(
        scan.stats.runtime.provenance(),
        crate::model::Provenance::Incomplete
    );
    assert!(!scan.deletion_eligible());
    assert!(crate::recycle::Target::from_scan(&scan, 0).is_err());
    let (volume, geometry) = external_list_volume();
    assert!(
        bootstrap_fixture(&mut Cursor::new(volume), geometry, &progress)
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn external_reads_do_not_corrupt_the_next_eight_megabyte_bulk_chunk() {
    use std::io::{Seek, SeekFrom};
    let (mut volume, mut geometry) = external_list_volume();
    add_external_file_list(&mut volume, false);
    let list_bytes = volume[5 * 4096..7 * 4096].to_vec();
    let mut flat = [
        volume[10 * 4096..12 * 4096].to_vec(),
        volume[20 * 4096..34 * 4096].to_vec(),
    ]
    .concat();
    flat.resize(CHUNK_BYTES + 65536, 0);
    let clusters = flat.len() / 4096;
    let mut data = nonresident(
        "",
        flat.len() as u64,
        flat.len() as u64,
        0,
        0,
        1,
        &[0x11, 2, 10, 0],
    );
    put16(&mut data, 14, 2);
    let mut name = filename(5, "$MFT", 1);
    put16(&mut name, 14, 1);
    let mut list = external_list_attr(96, &[0x11, 1, 5, 0], 1);
    put16(&mut list, 14, 3);
    flat[..1024].copy_from_slice(&record(0, false, 0, vec![list, name, data]));
    let count = (clusters - 2) as u16;
    let mut extension = nonresident(
        "",
        0,
        0,
        0,
        2,
        clusters as u64 - 1,
        &[0x12, count as u8, (count >> 8) as u8, 100, 0],
    );
    put16(&mut extension, 14, 4);
    flat[42 * 1024..43 * 1024].copy_from_slice(&record(42, false, reference(0), vec![extension]));
    flat[8193 * 1024..8194 * 1024].copy_from_slice(&record(
        8193,
        false,
        0,
        vec![
            filename(5, "after-chunk.txt", 1),
            resident_attr(0x80, "", b"abc"),
        ],
    ));
    let total = 100 + clusters;
    let mut volume = vec![0; total * 4096];
    volume[5 * 4096..7 * 4096].copy_from_slice(&list_bytes);
    volume[10 * 4096..12 * 4096].copy_from_slice(&flat[..8192]);
    volume[100 * 4096..100 * 4096 + flat.len() - 8192].copy_from_slice(&flat[8192..]);
    geometry.valid = flat.len() as u64;
    geometry.total_clusters = Some(total as u64);
    let progress = Progress::default();
    let mut disk = SectorVolume {
        disk: Cursor::new(volume),
        progress: &progress,
        list_reads: 0,
        cancel_second_list_read: false,
    };
    let mut first = flat[..1024].to_vec();
    fixup(&mut first, 512, false).unwrap();
    let mapping = mapping_from_record(
        &mut disk,
        &first,
        geometry,
        total as u64,
        &progress,
        |disk, reference| {
            let offset = 100 * 4096 + slot(reference) * 1024 - 8192;
            disk.seek(SeekFrom::Start(offset as u64))
                .map_err(|e| e.to_string())?;
            let mut raw = vec![0; 1024];
            disk.read_exact(&mut raw).map_err(|e| e.to_string())?;
            Ok(raw)
        },
    )
    .unwrap();
    let scan = scan_volume_fixture(&mut disk, mapping, geometry, &progress).unwrap();
    assert_eq!(scan.stats.mft_records, 8256);
    assert_eq!(scan.stats.files, 11);
    assert!(scan.nodes.iter().any(|node| node.name == "after-chunk.txt"));
    assert_eq!(scan.nodes[0].allocated, 94208 + CHUNK_BYTES as u64);
    assert_eq!(scan.nodes[0].logical, 1141328 + CHUNK_BYTES as u64 + 3);
}

#[test]
fn flat_replay_cannot_fetch_external_lists_from_the_displayed_drive() {
    let (mut volume, _) = external_list_volume();
    add_external_file_list(&mut volume, false);
    let mut flat = fixture(); // record-zero geometry is inferable without external bytes
    let offset = fixture_record_offset(25);
    flat[25 * 1024..26 * 1024].copy_from_slice(&volume[offset..offset + 1024]);
    let mut file = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut file, &flat).unwrap();
    let error = scan_file(Path::new("C:\\"), file.path(), &Progress::default()).unwrap_err();
    assert!(
        error.contains("flat saved MFT") && error.contains("external volume clusters"),
        "{error}"
    );
    let offset = fixture_record_offset(0);
    flat[..1024].copy_from_slice(&volume[offset..offset + 1024]);
    std::fs::write(file.path(), flat).unwrap();
    let error = scan_file(Path::new("C:\\"), file.path(), &Progress::default()).unwrap_err();
    assert!(
        error.contains("record-zero attribute list") && error.contains("external volume clusters"),
        "{error}"
    );
}

#[test]
fn resolved_list_snapshot_and_csv_round_trip_does_not_preserve_deletion_authority() {
    let (mut volume, geometry) = external_list_volume();
    add_external_file_list(&mut volume, false);
    let mut scan = scan_external_list_volume(volume, geometry).unwrap();
    scan.stats.volume_serial = Some(123);
    scan.stats.runtime = crate::model::RuntimeState::new(crate::model::Provenance::Live);
    assert!(scan.deletion_eligible());
    let output = tempfile::tempdir().unwrap();
    let snapshot = output.path().join("synthetic.json");
    crate::output::snapshot(&scan, &snapshot, &Progress::default()).unwrap();
    let loaded: Scan = serde_json::from_slice(&std::fs::read(snapshot).unwrap()).unwrap();
    assert_eq!(loaded.nodes[0].allocated, 94208);
    assert_eq!(
        loaded.stats.runtime.provenance(),
        crate::model::Provenance::Snapshot
    );
    assert!(!loaded.deletion_eligible());
    assert!(crate::recycle::Target::from_scan(&loaded, 1).is_err());
    let csv = output.path().join("synthetic.csv");
    crate::output::csv(&loaded, &csv, &Progress::default()).unwrap();
    let mut rows = csv::Reader::from_path(csv).unwrap();
    assert_eq!(rows.records().count(), loaded.nodes.len());
}
#[test]
fn rejects_reused_parent_and_extension_references() {
    let mut bytes = fixture();
    let b = record(30, true, 0, vec![filename(31, "Artifacts", 1)]);
    bytes[30 * 1024..31 * 1024].copy_from_slice(&b);
    put16(&mut bytes, 31 * 1024 + 16, 2);
    assert!(scan(bytes).unwrap_err().contains("parent"));
    let mut bytes = fixture();
    put64(&mut bytes, 26 * 1024 + 32, (2 << 48) | 25);
    assert!(scan(bytes).is_err());
}

#[test]
fn volume_parent_failure_reports_record_identities_without_private_names() {
    let (mut volume, geometry) = external_list_volume();
    put16(&mut volume, fixture_record_offset(31) + 16, 2);
    let error = scan_external_list_volume(volume, geometry).unwrap_err();
    eprintln!("Parent-reference failure: {error}");
    assert!(error.contains("child record 30, sequence 1"), "{error}");
    assert!(
        error.contains("expected parent record 31, sequence 1"),
        "{error}"
    );
    assert!(error.contains("scanned parent sequence 2"), "{error}");
    assert!(!error.contains("Projects") && !error.contains("Artifacts"));
}
fn volume_attempt(
    volume: Vec<u8>,
    geometry: Geometry,
    progress: &Progress,
) -> Result<Scan, IndexError> {
    let mut disk = Cursor::new(volume);
    let mapping = bootstrap_fixture(&mut disk, geometry, progress)?;
    let reader = MappedReader::with_bootstrap(&mut disk, mapping, geometry)?;
    read_index_resolved(
        reader,
        Path::new("C:\\synthetic-fixture"),
        geometry,
        progress,
        "Volume fixture",
    )
}

#[test]
fn live_parent_retry_restarts_the_complete_validated_pipeline_without_double_counting() {
    let (volume, geometry) = external_list_volume();
    let mut changed = volume.clone();
    put16(&mut changed, fixture_record_offset(31) + 16, 2);
    let progress = Progress::default();
    let mut attempts = 0;
    let scan = scan_live_with_retry(Path::new("C:\\synthetic-fixture"), &progress, || {
        attempts += 1;
        if attempts == 2 {
            for counter in [
                &progress.files,
                &progress.folders,
                &progress.bytes,
                &progress.metadata_bytes,
                &progress.metadata_total,
                &progress.mft_records,
            ] {
                assert_eq!(counter.load(Ordering::Relaxed), 0);
            }
            assert!(!progress.cancelled());
        }
        volume_attempt(
            if attempts == 1 {
                changed.clone()
            } else {
                volume.clone()
            },
            geometry,
            &progress,
        )
    })
    .unwrap();
    assert_eq!(attempts, 2);
    assert_eq!(
        (
            scan.stats.files,
            scan.stats.folders,
            scan.stats.shared_links
        ),
        (10, 3, 1)
    );
    assert_eq!(scan.stats.mft_records, 64);
    assert_eq!(
        (scan.nodes[0].logical, scan.nodes[0].allocated),
        (1141328, 94208)
    );
    assert!(
        scan.stats
            .notices
            .iter()
            .any(|notice| notice.contains("second read passed reference validation"))
    );
    assert!(!scan.deletion_eligible());
}

#[test]
fn persistent_parent_mismatch_and_wrong_parent_type_fail_after_one_retry() {
    for wrong_type in [false, true] {
        let (mut volume, geometry) = external_list_volume();
        if wrong_type {
            put_volume_record(
                &mut volume,
                31,
                &record(
                    31,
                    false,
                    0,
                    vec![filename(5, "Projects", 1), resident_attr(0x80, "", &[])],
                ),
            );
        } else {
            put16(&mut volume, fixture_record_offset(31) + 16, 2);
        }
        let progress = Progress::default();
        let mut attempts = 0;
        let error = scan_live_with_retry(Path::new("C:\\synthetic-fixture"), &progress, || {
            attempts += 1;
            volume_attempt(volume.clone(), geometry, &progress)
        })
        .unwrap_err();
        assert_eq!(attempts, 2);
        assert!(
            error.contains("child record 30, sequence 1")
                && error.contains("expected parent record 31, sequence 1"),
            "{error}"
        );
        assert!(error.contains("fresh-index retry also failed"));
        assert!(
            error.contains(if wrong_type {
                "not a directory"
            } else {
                "scanned parent sequence 2"
            }),
            "{error}"
        );
        assert!(!error.contains("Projects") && !error.contains("Artifacts"));
    }
}

#[test]
fn successful_index_and_fatal_metadata_errors_never_trigger_the_parent_retry() {
    for fatal in [false, true] {
        let (mut volume, geometry) = external_list_volume();
        if fatal {
            let offset = fixture_record_offset(20);
            volume[offset..offset + 4].copy_from_slice(b"BAAD");
        }
        let progress = Progress::default();
        let mut attempts = 0;
        let result = scan_live_with_retry(Path::new("C:\\synthetic-fixture"), &progress, || {
            attempts += 1;
            volume_attempt(volume.clone(), geometry, &progress)
        });
        assert_eq!(attempts, 1);
        if fatal {
            assert!(
                result
                    .unwrap_err()
                    .contains("Unrecognized NTFS record at index 20")
            );
        } else {
            assert_eq!(result.unwrap().nodes[0].allocated, 94208);
        }
    }
}

#[test]
fn cancellation_after_a_parent_failure_does_not_restart_or_authorize_deletion() {
    let (mut volume, geometry) = external_list_volume();
    put16(&mut volume, fixture_record_offset(31) + 16, 2);
    let progress = Progress::default();
    let mut attempts = 0;
    let scan = scan_live_with_retry(Path::new("C:\\synthetic-fixture"), &progress, || {
        attempts += 1;
        let result = volume_attempt(volume.clone(), geometry, &progress);
        progress.cancel.store(true, Ordering::Relaxed);
        result
    })
    .unwrap();
    assert_eq!(attempts, 1);
    assert!(scan.stats.cancelled && progress.cancelled());
    assert_eq!(
        scan.stats.runtime.provenance(),
        crate::model::Provenance::Incomplete
    );
    assert!(!scan.deletion_eligible());
    assert!(crate::recycle::Target::from_scan(&scan, 0).is_err());
}

#[test]
fn saved_mft_parent_errors_keep_diagnostics_without_reading_or_retrying_a_live_drive() {
    let mut bytes = fixture();
    put16(&mut bytes, 31 * 1024 + 16, 2);
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), bytes).unwrap();
    let error = scan_file(Path::new("C:\\"), file.path(), &Progress::default()).unwrap_err();
    assert!(
        error.contains("child record 30, sequence 1")
            && error.contains("scanned parent sequence 2"),
        "{error}"
    );
    assert!(!error.contains("fresh-index retry"));
}

#[test]
fn rejects_disconnected_directory_cycle() {
    let mut bytes = fixture();
    for (i, parent) in [(30, 31), (31, 30)] {
        let b = record(i, true, 0, vec![filename(parent, "cycle", 1)]);
        bytes[i as usize * 1024..(i as usize + 1) * 1024].copy_from_slice(&b);
    }
    assert!(scan(bytes).unwrap_err().contains("cyclic"));
}
#[test]
fn rejects_unknown_active_slot_instead_of_silent_undercount() {
    let mut bytes = fixture();
    bytes[40 * 1024..40 * 1024 + 4].copy_from_slice(b"BAAD");
    assert!(scan(bytes).unwrap_err().contains("Unrecognized"));
}

#[test]
fn zero_link_unnamed_records_are_reported_without_inventing_paths() {
    let mut bytes = fixture();
    let b = record(
        40,
        false,
        0,
        vec![nonresident("", 736, 4096, 0, 0, 0, &[0x11, 1, 70, 0])],
    );
    bytes[40 * 1024..41 * 1024].copy_from_slice(&b);
    let result = scan(bytes.clone()).unwrap();
    assert_eq!((result.stats.files, result.nodes[0].allocated), (10, 94208));
    assert_eq!(
        (
            result.stats.unlinked_records,
            result.stats.unlinked_logical_bytes,
            result.stats.unlinked_allocated_bytes
        ),
        (1, 736, 4096)
    );
    put16(&mut bytes, 40 * 1024 + 18, 1);
    assert!(scan(bytes).unwrap_err().contains("record 40 has 1 links"));
}

#[test]
fn holes_without_sparse_flag_do_not_count_the_virtual_span_as_allocation() {
    let mut bytes = fixture();
    let b = record(
        40,
        false,
        0,
        vec![
            filename(5, "hole-index", 1),
            resident_attr(0x80, "", &[]),
            nonresident("range", 4194304, 4194304, 0, 0, 1023, &[0x02, 0, 4, 0]),
        ],
    );
    bytes[40 * 1024..41 * 1024].copy_from_slice(&b);
    let result = scan(bytes).unwrap();
    let node = result
        .nodes
        .iter()
        .find(|n| n.name == "hole-index")
        .unwrap();
    assert_eq!((node.logical, node.allocated), (0, 0));
    assert_eq!(result.nodes[0].allocated, 94208);
}

#[test]
fn physical_runs_are_summed_across_data_extension_segments() {
    let mut bytes = fixture();
    let base = record(
        40,
        false,
        0,
        vec![
            filename(5, "fragmented.bin", 1),
            nonresident("", 16384, 16384, 0, 0, 1, &[0x11, 1, 70, 0x01, 1, 0]),
        ],
    );
    let extension = record(
        41,
        false,
        reference(40),
        vec![nonresident("", 0, 0, 0, 2, 3, &[0x11, 1, 80, 0x01, 1, 0])],
    );
    bytes[40 * 1024..41 * 1024].copy_from_slice(&base);
    bytes[41 * 1024..42 * 1024].copy_from_slice(&extension);
    let result = scan(bytes).unwrap();
    let node = result
        .nodes
        .iter()
        .find(|n| n.name == "fragmented.bin")
        .unwrap();
    assert_eq!((node.logical, node.allocated), (16384, 8192));
}
#[test]
fn rejects_unsafe_file_names() {
    for name in ["..", "a\\b", "C:escape", "", "."] {
        let b = record(40, false, 0, vec![filename(5, name, 1)]);
        let mut bytes = fixture();
        bytes[40 * 1024..41 * 1024].copy_from_slice(&b);
        assert!(scan(bytes).is_err(), "{name}");
    }
}
#[test]
fn cancellation_returns_explicit_partial_mft_tree() {
    let progress = Progress::default();
    progress.cancel.store(true, Ordering::Relaxed);
    let bytes = fixture();
    let result = read_index(
        Cursor::new(&bytes),
        Path::new("C:\\fixture"),
        geometry(bytes.len()),
        &progress,
        "fixture",
    )
    .unwrap();
    assert!(result.stats.cancelled);
    assert_eq!(
        result.stats.runtime.provenance(),
        crate::model::Provenance::Incomplete
    );
    assert!(!result.deletion_eligible());
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.stats.mft_records, 0);
}
#[test]
fn cancellation_between_bulk_reads_prunes_unread_parents() {
    let progress = Progress::default();
    let mut bytes = fixture();
    bytes.resize(10000 * 1024, 0);
    let b = record(40, false, 0, vec![filename(9000, "unreachable.txt", 1)]);
    bytes[40 * 1024..41 * 1024].copy_from_slice(&b);
    struct CancelReader<'a> {
        inner: Cursor<Vec<u8>>,
        progress: &'a Progress,
    }
    impl Read for CancelReader<'_> {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            if self.inner.position() >= CHUNK_BYTES as u64 {
                self.progress.cancel.store(true, Ordering::Relaxed);
            }
            self.inner.read(b)
        }
    }
    let len = bytes.len();
    let result = read_index(
        CancelReader {
            inner: Cursor::new(bytes),
            progress: &progress,
        },
        Path::new("C:\\fixture"),
        geometry(len),
        &progress,
        "fixture",
    )
    .unwrap();
    assert!(result.stats.cancelled);
    assert_eq!(result.stats.files, 10);
    assert!(!result.nodes.iter().any(|n| n.name == "unreachable.txt"));
    assert_eq!(result.stats.errors, 1);
    assert_eq!(result.stats.mft_records, 8192);
}
