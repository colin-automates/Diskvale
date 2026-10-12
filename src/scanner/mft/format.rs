use std::collections::HashSet;
pub(super) const REFERENCE_MASK: u64 = 0x0000_ffff_ffff_ffff;
pub(super) fn slot(reference: u64) -> usize {
    (reference & REFERENCE_MASK) as usize
}
pub(super) fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(i..i.checked_add(2)?)?.try_into().ok()?,
    ))
}
pub(super) fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(i..i.checked_add(4)?)?.try_into().ok()?,
    ))
}
pub(super) fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(i..i.checked_add(8)?)?.try_into().ok()?,
    ))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Run {
    pub lcn: Option<u64>,
    pub clusters: u64,
}
pub(super) fn runs(bytes: &[u8]) -> Result<Vec<Run>, String> {
    let mut result = vec![];
    let (mut i, mut lcn) = (0usize, 0i64);
    while let Some(&head) = bytes.get(i) {
        i += 1;
        if head == 0 {
            return Ok(result);
        }
        let (n, m) = ((head & 15) as usize, (head >> 4) as usize);
        if n == 0 || n > 8 || m > 8 || i + n + m > bytes.len() {
            return Err("Invalid NTFS mapping pair".into());
        }
        let mut length = [0; 8];
        length[..n].copy_from_slice(&bytes[i..i + n]);
        i += n;
        let clusters = u64::from_le_bytes(length);
        if clusters == 0 {
            return Err("Empty NTFS run".into());
        }
        let physical = if m == 0 {
            None
        } else {
            let mut delta = if bytes[i + m - 1] & 0x80 != 0 {
                [255; 8]
            } else {
                [0; 8]
            };
            delta[..m].copy_from_slice(&bytes[i..i + m]);
            i += m;
            lcn = lcn
                .checked_add(i64::from_le_bytes(delta))
                .ok_or("NTFS run overflow")?;
            if lcn < 0 {
                return Err("Negative NTFS cluster".into());
            }
            Some(lcn as u64)
        };
        result.push(Run {
            lcn: physical,
            clusters,
        });
    }
    Err("Unterminated NTFS mapping pairs".into())
}
pub(super) fn fixup(record: &mut [u8], sector: usize, kernel_record: bool) -> Result<(), String> {
    if record.get(..4) != Some(b"FILE") {
        return Err("Missing FILE signature".into());
    }
    let offset = u16_at(record, 4).ok_or("Missing update sequence")? as usize;
    let count = u16_at(record, 6).ok_or("Missing sequence count")? as usize;
    if sector < 256
        || !sector.is_power_of_two()
        || !record.len().is_multiple_of(sector)
        || count != record.len() / sector + 1
        || offset < 42
        || !offset.is_multiple_of(2)
    {
        return Err("Unsupported NTFS record geometry".into());
    }
    let usa = record
        .get(offset..offset + count * 2)
        .ok_or("Truncated update sequence")?;
    if let Ok(saved) = <[u8; 6]>::try_from(usa) {
        // The common 1 KiB record / 512-byte sector geometry needs six bytes.
        return restore_tails(record, &saved, sector, kernel_record);
    }
    // All supported geometries fit here (64 KiB records, >=256-byte sectors).
    // Copy before restoring tails, including unusual overlapping USA layouts.
    let mut scratch = [0u8; 514];
    if usa.len() <= scratch.len() {
        let len = usa.len();
        scratch[..len].copy_from_slice(usa);
        restore_tails(record, &scratch[..len], sector, kernel_record)
    } else {
        // Preserve this helper's behavior outside the bounded live geometry.
        let saved = usa.to_vec();
        restore_tails(record, &saved, sector, kernel_record)
    }
}
fn restore_tails(
    record: &mut [u8],
    usa: &[u8],
    sector: usize,
    kernel_record: bool,
) -> Result<(), String> {
    let count = usa.len() / 2;
    let raw = (1..count).all(|i| record[i * sector - 2..i * sector] == usa[..2]);
    // Kernel metadata queries can return restored tails; raw bulk reads must match every signature.
    let restored = kernel_record
        && (1..count).all(|i| record[i * sector - 2..i * sector] == usa[i * 2..i * 2 + 2]);
    if !raw && !restored {
        return Err("NTFS record changed during read (sector sequence mismatch)".into());
    }
    if raw {
        for i in 1..count {
            record[i * sector - 2..i * sector].copy_from_slice(&usa[i * 2..i * 2 + 2]);
        }
    }
    Ok(())
}
pub(super) fn attributes(record: &[u8]) -> Result<Vec<&[u8]>, String> {
    Ok(attribute_view(record)?.collect())
}

#[derive(Clone)]
struct Attributes<'a> {
    record: &'a [u8],
    offset: usize,
    end: usize,
    has_list: bool,
}
impl<'a> Iterator for Attributes<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.end {
            return None;
        }
        // The complete immutable record was validated before exposing this view.
        let len = u32_at(self.record, self.offset + 4).expect("validated attribute") as usize;
        let attr = &self.record[self.offset..self.offset + len];
        self.offset += len;
        Some(attr)
    }
}
fn attribute_view(record: &[u8]) -> Result<Attributes<'_>, String> {
    let mut offset = u16_at(record, 20).ok_or("Missing attribute offset")? as usize;
    let used = u32_at(record, 24).ok_or("Missing record length")? as usize;
    let allocated = u32_at(record, 28).ok_or("Missing allocated record length")? as usize;
    let usa_end =
        u16_at(record, 4).unwrap_or(0) as usize + u16_at(record, 6).unwrap_or(0) as usize * 2;
    if used > record.len()
        || allocated != record.len()
        || offset < 48
        || offset < usa_end
        || !offset.is_multiple_of(8)
        || offset > used
    {
        return Err("Invalid FILE record bounds".into());
    }
    let first = offset;
    let mut has_list = false;
    while offset + 4 <= used {
        let typ = u32_at(record, offset).ok_or("Missing attribute type")?;
        if typ == u32::MAX {
            return Ok(Attributes {
                record,
                offset: first,
                end: offset,
                has_list,
            });
        }
        let len = u32_at(record, offset + 4).ok_or("Missing attribute size")? as usize;
        if len < 24 || !len.is_multiple_of(8) || len > used - offset {
            return Err("Invalid attribute length".into());
        }
        let attr = &record[offset..offset + len];
        if attr[8] > 1 {
            return Err("Invalid attribute form".into());
        }
        if attr[8] == 1 && len < 64 {
            return Err("Truncated nonresident attribute".into());
        }
        if attr[8] == 0 {
            resident(attr)?;
        }
        validate_unicode(attribute_name_bytes(attr)?)?;
        has_list |= typ == 0x20;
        offset += len;
    }
    Err("Missing attribute terminator".into())
}
pub(super) fn resident(attr: &[u8]) -> Result<&[u8], String> {
    if attr.get(8) != Some(&0) {
        return Err("Expected resident attribute".into());
    }
    let len = u32_at(attr, 16).ok_or("Missing resident length")? as usize;
    let off = u16_at(attr, 20).ok_or("Missing resident offset")? as usize;
    if off < 24 {
        return Err("Resident data overlaps its header".into());
    }
    attr.get(off..off.checked_add(len).ok_or("Resident length overflow")?)
        .ok_or_else(|| "Invalid resident data bounds".into())
}
fn unicode(bytes: &[u8]) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("Unaligned UTF-16 name".into());
    }
    if bytes
        .as_chunks::<2>()
        .0
        .iter()
        .all(|b| b[1] == 0 && b[0] < 128)
    {
        // This allocation becomes the String's storage; there is no UTF-16 Vec.
        let ascii = bytes.as_chunks::<2>().0.iter().map(|b| b[0]).collect();
        return Ok(String::from_utf8(ascii).expect("validated ASCII"));
    }
    let mut text = String::with_capacity(bytes.len() / 2);
    for character in char::decode_utf16(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b)),
    ) {
        text.push(character.map_err(|_| "Unsupported invalid UTF-16 NTFS name")?);
    }
    Ok(text)
}
fn validate_unicode(bytes: &[u8]) -> Result<(), String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("Unaligned UTF-16 name".into());
    }
    if char::decode_utf16(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b)),
    )
    .any(|character| character.is_err())
    {
        return Err("Unsupported invalid UTF-16 NTFS name".into());
    }
    Ok(())
}
pub(super) fn attribute_name(attr: &[u8]) -> Result<String, String> {
    unicode(attribute_name_bytes(attr)?)
}
fn attribute_name_bytes(attr: &[u8]) -> Result<&[u8], String> {
    let chars = *attr.get(9).ok_or("Missing attribute name length")? as usize;
    if chars == 0 {
        return Ok(&[]);
    }
    let off = u16_at(attr, 10).ok_or("Missing attribute name offset")? as usize;
    if off < if attr[8] == 0 { 24 } else { 64 } {
        return Err("Attribute name overlaps its header".into());
    }
    attr.get(off..off + chars * 2)
        .ok_or_else(|| "Invalid stream name bounds".into())
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ListEntry {
    pub typ: u32,
    pub vcn: u64,
    pub reference: u64,
    pub instance: u16,
    pub name: String,
}
pub(super) fn attribute_list(data: &[u8]) -> Result<Vec<ListEntry>, String> {
    let mut result = vec![];
    let mut off = 0;
    while off < data.len() {
        if data[off..].iter().all(|&v| v == 0) {
            break;
        }
        let typ = u32_at(data, off).ok_or("Truncated attribute list")?;
        if typ == u32::MAX {
            break;
        }
        let len = u16_at(data, off + 4).ok_or("Missing attribute list length")? as usize;
        if len < 26 || len > data.len() - off {
            return Err("Invalid attribute list entry".into());
        }
        let entry = &data[off..off + len];
        let chars = entry[6] as usize;
        let name_off = entry[7] as usize;
        let name = if chars == 0 {
            String::new()
        } else {
            if name_off < 26 {
                return Err("Attribute list name overlaps its header".into());
            }
            unicode(
                entry
                    .get(name_off..name_off + chars * 2)
                    .ok_or("Invalid attribute list name")?,
            )?
        };
        result.push(ListEntry {
            typ,
            vcn: u64_at(entry, 8).ok_or("Missing list VCN")?,
            reference: u64_at(entry, 16).ok_or("Missing list reference")?,
            instance: u16_at(entry, 24).ok_or("Missing list instance")?,
            name,
        });
        off += len;
    }
    Ok(result)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Name {
    pub parent: u64,
    pub text: String,
    pub dos: bool,
}
#[derive(Debug, Clone)]
pub(super) struct Stream {
    pub name: String,
    pub logical: u64,
    pub allocated: u64,
    pub range: Option<(u64, u64)>,
    pub extent: u64,
}
#[derive(Default, Debug)]
pub(super) struct Record {
    pub cluster: u64,
    pub reference: u64,
    pub base: u64,
    pub directory: bool,
    pub links: u16,
    pub names: Vec<Name>,
    pub data: Option<(u64, u64)>,
    pub data_range: Option<(u64, u64)>,
    pub data_extent: u64,
    pub named: Vec<Stream>,
    pub continuations: Vec<Stream>,
    pub incomplete: bool,
    pub required: Vec<ListEntry>,
    pub present: Vec<ListEntry>,
    pub modified: u64,
    pub reparse: bool,
}
impl Record {
    fn add_stream(&mut self, stream: Stream) -> Result<(), String> {
        if stream.name.is_empty() {
            if self
                .data
                .replace((stream.logical, stream.allocated))
                .is_some()
            {
                return Err("Duplicate unnamed NTFS stream header".into());
            }
            self.data_range = stream.range;
            self.data_extent = stream.extent;
        } else {
            if self.named.iter().any(|s| s.name == stream.name) {
                return Err("Duplicate named NTFS stream header".into());
            }
            self.named.push(stream);
        }
        Ok(())
    }
    pub fn merge(&mut self, other: Self) -> Result<(), String> {
        if self.directory != other.directory {
            return Err("NTFS extension type changed".into());
        }
        if let Some((logical, allocated)) = other.data {
            self.add_stream(Stream {
                name: String::new(),
                logical,
                allocated,
                range: other.data_range,
                extent: other.data_extent,
            })?;
        }
        for stream in other.named {
            self.add_stream(stream)?;
        }
        self.names.extend(other.names);
        self.continuations.extend(other.continuations);
        self.present.extend(other.present);
        self.required.extend(other.required);
        self.reparse |= other.reparse;
        Ok(())
    }
    pub fn finalize_streams(&mut self, cancelled: bool) -> Result<(), String> {
        if !self.required.is_empty() {
            let present: HashSet<_> = self.present.iter().collect();
            if self.required.iter().any(|entry| !present.contains(entry)) {
                if !cancelled {
                    return Err("Missing or mismatched listed NTFS attribute".into());
                }
                self.incomplete = true;
            }
        }
        self.continuations
            .sort_unstable_by(|a, b| (&a.name, a.range).cmp(&(&b.name, b.range)));
        for part in &self.continuations {
            let (allocation, range) = if part.name.is_empty() {
                if let Some(data) = &mut self.data {
                    (&mut data.1, &mut self.data_range)
                } else {
                    if cancelled {
                        self.incomplete = true;
                        continue;
                    }
                    return Err("Missing NTFS stream base attribute".into());
                }
            } else if let Some(base) = self.named.iter_mut().find(|s| s.name == part.name) {
                (&mut base.allocated, &mut base.range)
            } else {
                if cancelled {
                    self.incomplete = true;
                    continue;
                }
                return Err("Missing NTFS named-stream base attribute".into());
            };
            let contiguous = range
                .as_ref()
                .and_then(|(_, highest)| highest.checked_add(1))
                .is_some_and(|next| part.range.is_some_and(|(low, _)| low == next));
            if !contiguous {
                if cancelled {
                    self.incomplete = true;
                    continue;
                }
                return Err("Missing or overlapping NTFS data extent segment".into());
            }
            *allocation = allocation
                .checked_add(part.allocated)
                .ok_or("NTFS stream allocation overflow")?;
            if let (Some(base), Some(part)) = (range.as_mut(), part.range) {
                base.1 = part.1;
            }
        }
        let mut extents = vec![];
        if let Some((logical, _)) = self.data {
            extents.push((logical.max(self.data_extent), self.data_range));
        }
        extents.extend(
            self.named
                .iter()
                .map(|s| (s.logical.max(s.extent), s.range)),
        );
        for (logical, range) in extents {
            if let Some((low, high)) = range {
                // Virtual coverage includes sparse holes, compression padding and preallocation.
                // It is deliberately independent of physically mapped allocation.
                if low != 0
                    || (logical != 0
                        && high
                            .checked_add(1)
                            .and_then(|n| n.checked_mul(self.cluster))
                            .is_none_or(|n| n < logical))
                {
                    if !cancelled {
                        return Err("Incomplete terminal NTFS stream coverage".into());
                    }
                    self.incomplete = true;
                }
            }
        }
        Ok(())
    }
    pub fn named_logical(&self) -> Result<u64, String> {
        self.named.iter().try_fold(0u64, |sum, s| {
            sum.checked_add(s.logical)
                .ok_or_else(|| "Named stream size overflow".into())
        })
    }
    pub fn allocation(&self) -> Result<u64, String> {
        self.named
            .iter()
            .try_fold(self.data.map_or(0, |s| s.1), |sum, s| {
                sum.checked_add(s.allocated)
                    .ok_or_else(|| "NTFS allocation overflow".into())
            })
    }
    pub fn normalize_names(&mut self) {
        if self.names.len() <= 1 {
            return;
        }
        if self.names.len() == 2
            && self.names[0].parent == self.names[1].parent
            && self.names[0].dos != self.names[1].dos
        {
            // A DOS alias and its long name share a parent. Other hard-link
            // combinations still use full normalization and deduplication.
            let dos = usize::from(!self.names[0].dos);
            self.names.swap_remove(dos);
            return;
        }
        let long_parents: HashSet<_> = self
            .names
            .iter()
            .filter(|n| !n.dos)
            .map(|n| n.parent)
            .collect();
        self.names
            .retain(|n| !n.dos || !long_parents.contains(&n.parent));
        self.names
            .sort_unstable_by(|a, b| (a.parent, &a.text).cmp(&(b.parent, &b.text)));
        self.names
            .dedup_by(|a, b| a.parent == b.parent && a.text == b.text);
    }
}
#[cfg(test)]
pub(super) fn parse(
    raw: &[u8],
    index: u64,
    cluster: u64,
    total_clusters: Option<u64>,
) -> Result<Option<Record>, String> {
    parse_with_lists(raw, index, cluster, total_clusters, |_| {
        Err(
            "Unsupported nonresident attribute list: external volume metadata is unavailable"
                .into(),
        )
    })
}

pub(super) fn parse_with_lists(
    raw: &[u8],
    index: u64,
    cluster: u64,
    total_clusters: Option<u64>,
    mut resolve_list: impl FnMut(&[u8]) -> Result<Vec<ListEntry>, String>,
) -> Result<Option<Record>, String> {
    let flags = u16_at(raw, 22).ok_or("Missing record flags")?;
    if flags & 1 == 0 {
        return Ok(None);
    }
    if index > REFERENCE_MASK {
        return Err("NTFS record index overflow".into());
    }
    let mut record = Record {
        cluster,
        reference: index | ((u16_at(raw, 16).ok_or("Missing record sequence")? as u64) << 48),
        base: u64_at(raw, 32).ok_or("Missing base reference")?,
        directory: flags & 2 != 0,
        links: u16_at(raw, 18).ok_or("Missing NTFS link count")?,
        ..Default::default()
    };
    let attrs = attribute_view(raw)?;
    let needs_identities = record.base != 0 || attrs.has_list;
    for attr in attrs {
        if needs_identities {
            record.present.push(ListEntry {
                typ: u32_at(attr, 0).ok_or("Missing attribute type")?,
                vcn: if attr[8] == 1 {
                    u64_at(attr, 16).ok_or("Missing attribute VCN")?
                } else {
                    0
                },
                reference: record.reference,
                instance: u16_at(attr, 14).ok_or("Missing attribute instance")?,
                name: attribute_name(attr)?,
            });
        }
        match u32_at(attr, 0).unwrap_or(0) {
            0x10 => {
                if attr[8] == 0 {
                    record.modified = u64_at(resident(attr)?, 8)
                        .unwrap_or(0)
                        .saturating_sub(116444736000000000)
                        / 10000000;
                }
            }
            0x20 => {
                if attr[8] == 0 {
                    record.required.extend(attribute_list(resident(attr)?)?);
                } else {
                    record.required.extend(resolve_list(attr)?);
                }
            }
            0x30 => {
                let name = resident(attr)?;
                if name.len() < 66 || name[65] > 3 {
                    return Err("Invalid NTFS file name".into());
                }
                let chars = name[64] as usize;
                let text = unicode(
                    name.get(66..66 + chars * 2)
                        .ok_or("Invalid file name length")?,
                )?;
                if text.is_empty()
                    || text.contains(['\\', '/', '\0', ':'])
                    || text == ".."
                    || (text == "." && index != 5)
                {
                    return Err("Invalid NTFS path component".into());
                }
                if record.names.capacity() == 0 {
                    // Ordinary records have one long name and often a DOS alias.
                    record.names.reserve_exact(2);
                }
                record.names.push(Name {
                    parent: u64_at(name, 0).ok_or("Missing parent reference")?,
                    text,
                    dos: name[65] == 2,
                });
            }
            0x80 => {
                let name = attribute_name(attr)?;
                if attr[8] == 0 {
                    record.add_stream(Stream {
                        name,
                        logical: resident(attr)?.len() as u64,
                        allocated: 0,
                        range: None,
                        extent: 0,
                    })?;
                } else {
                    let low = u64_at(attr, 16).ok_or("Missing stream VCN")?;
                    let high = u64_at(attr, 24).ok_or("Missing stream end VCN")?;
                    let offset = u16_at(attr, 32).ok_or("Missing data mapping offset")? as usize;
                    if offset < 64 || cluster == 0 {
                        return Err("Invalid data mapping geometry".into());
                    }
                    let mapping = runs(attr.get(offset..).ok_or("Invalid data mapping offset")?)?;
                    for run in &mapping {
                        if let Some(lcn) = run.lcn {
                            let end = lcn
                                .checked_add(run.clusters)
                                .ok_or("Physical extent overflow")?;
                            if total_clusters.is_some_and(|total| end > total) {
                                return Err("NTFS data extent exceeds the volume".into());
                            }
                            end.checked_mul(cluster)
                                .ok_or("Physical data extent overflow")?;
                        }
                    }
                    let covered = mapping
                        .iter()
                        .try_fold(0u64, |sum, r| sum.checked_add(r.clusters))
                        .ok_or("Data VCN overflow")?;
                    if covered != 0
                        && high.checked_sub(low).and_then(|n| n.checked_add(1)) != Some(covered)
                    {
                        return Err("Data mapping does not match its VCN range".into());
                    }
                    let physical = mapping
                        .iter()
                        .filter(|r| r.lcn.is_some())
                        .try_fold(0u64, |sum, r| sum.checked_add(r.clusters))
                        .and_then(|n| n.checked_mul(cluster))
                        .ok_or("Physical data allocation overflow")?;
                    let stream = Stream {
                        name,
                        logical: if low == 0 {
                            u64_at(attr, 48).ok_or("Missing stream size")?
                        } else {
                            0
                        },
                        allocated: physical,
                        range: (covered != 0).then_some((low, high)),
                        extent: if low == 0 {
                            u64_at(attr, 40).ok_or("Missing declared stream extent")?
                        } else {
                            0
                        },
                    };
                    if low == 0
                        && (stream.extent > i64::MAX as u64
                            || stream.logical > i64::MAX as u64
                            || !stream.extent.is_multiple_of(cluster))
                    {
                        return Err("Invalid declared NTFS stream extent".into());
                    }
                    if covered == 0 && stream.extent != 0 {
                        return Err("Missing preallocated data mapping runs".into());
                    }
                    if covered == 0 && (low != 0 || stream.logical != 0) {
                        return Err("Missing data mapping runs".into());
                    }
                    // Count physical runs for every stream, including NTFS system
                    // streams with holes but no sparse flag. Header spans are not allocation.
                    if low == 0 {
                        record.add_stream(stream)?;
                    } else {
                        record.continuations.push(stream);
                    }
                }
            }
            0xc0 => record.reparse = true,
            _ => {}
        }
    }
    Ok(Some(record))
}
