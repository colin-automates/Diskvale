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
        |_, r| {
            assert_eq!(r, reference(42));
            queries += 1;
            Ok(extension.clone())
        },
    )
    .unwrap();
    assert_eq!(queries, 1);
    assert_eq!(
        result,
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
        |_, _| Ok(extension.clone()),
    )
    .unwrap();
    assert_eq!(result.len(), 2);
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
