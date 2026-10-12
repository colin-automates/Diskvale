use super::{
    Progress,
    format::{self, ListEntry, Run},
};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

const MAX_LIST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BOOTSTRAP_BYTES: usize = 64 * 1024 * 1024;

/// Flat saved indexes have no authority or source for external volume clusters.
pub(super) trait IndexSource: Read {
    fn resolve_list(
        &mut self,
        _attr: &[u8],
        _geometry: Geometry,
        _progress: &Progress,
    ) -> Result<Vec<ListEntry>, String> {
        Err("Unsupported nonresident attribute list in flat saved MFT: external volume clusters are unavailable; use a live NTFS scan".into())
    }
    fn validate_record(&mut self, _index: usize, _raw: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn validate_parsed(&self, _record: &format::Record) -> Result<(), String> {
        Ok(())
    }
    fn finish_validation(&self) -> Result<(), String> {
        Ok(())
    }
}

pub(super) struct FlatIndex<R>(pub R);
impl<R: Read> Read for FlatIndex<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}
impl<R: Read> IndexSource for FlatIndex<R> {}

#[derive(Debug, PartialEq, Eq)]
struct BootstrapRecord {
    reference: u64,
    base: u64,
    flags: u16,
    attributes: Vec<Vec<u8>>,
}
impl BootstrapRecord {
    fn from_raw(index: usize, raw: &[u8]) -> Result<Self, String> {
        if raw.get(..4) != Some(b"FILE")
            || format::u16_at(raw, 22).is_none_or(|flags| flags & 1 == 0)
        {
            return Err(
                "$MFT record changed or disappeared since bootstrap; rescan the live volume".into(),
            );
        }
        Ok(Self {
            reference: index as u64
                | ((format::u16_at(raw, 16).ok_or("Missing record sequence")? as u64) << 48),
            base: format::u64_at(raw, 32).ok_or("Missing base reference")?,
            flags: format::u16_at(raw, 22).ok_or("Missing record flags")?,
            // Compare normalized attribute bytes, not LSN/update-sequence fields
            // that differ between raw reads and FSCTL-restored records.
            attributes: format::attributes(raw)?
                .into_iter()
                .map(<[u8]>::to_vec)
                .collect(),
        })
    }
}

#[derive(Debug)]
pub(super) struct Bootstrap {
    pub runs: Vec<Run>,
    records: HashMap<usize, BootstrapRecord>,
    required: Vec<ListEntry>,
    first_reference: u64,
}

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub record_size: usize,
    pub sector: usize,
    pub cluster: u64,
    pub valid: u64,
    pub total_clusters: Option<u64>,
}
impl Geometry {
    pub fn validate(self) -> Result<(), String> {
        if !(512..=65536).contains(&self.record_size)
            || !self.record_size.is_power_of_two()
            || self.sector < 256
            || !self.sector.is_power_of_two()
            || !self.record_size.is_multiple_of(self.sector)
            || self.valid == 0
            || !self.valid.is_multiple_of(self.record_size as u64)
        {
            return Err("Unsupported NTFS index geometry".into());
        }
        Ok(())
    }
}

pub(super) struct MappedReader<'a, R> {
    file: &'a mut R,
    runs: Vec<Run>,
    cluster: u64,
    remaining: u64,
    run: usize,
    in_run: u64,
    bootstrap: Option<Bootstrap>,
    cursor: Option<u64>,
    list_buffer: Vec<u8>,
}
impl<'a, R: Read + Seek> MappedReader<'a, R> {
    pub fn new(file: &'a mut R, runs: Vec<Run>, cluster: u64, valid: u64) -> Result<Self, String> {
        if cluster == 0 || runs.iter().any(|r| r.lcn.is_none() || r.clusters == 0) {
            return Err("Sparse or empty NTFS metadata mapping".into());
        }
        let mapped = runs
            .iter()
            .try_fold(0u64, |sum, r| {
                sum.checked_add(r.clusters.checked_mul(cluster)?)
            })
            .ok_or("NTFS mapping length overflow")?;
        if mapped < valid {
            return Err("Incomplete NTFS metadata mapping".into());
        }
        Ok(Self {
            file,
            runs,
            cluster,
            remaining: valid,
            run: 0,
            in_run: 0,
            bootstrap: None,
            cursor: None,
            list_buffer: Vec::new(),
        })
    }

    pub fn with_bootstrap(
        file: &'a mut R,
        mut bootstrap: Bootstrap,
        geometry: Geometry,
    ) -> Result<Self, String> {
        let mut reader = Self::new(
            file,
            std::mem::take(&mut bootstrap.runs),
            geometry.cluster,
            geometry.valid,
        )?;
        reader.bootstrap = Some(bootstrap);
        Ok(reader)
    }
}

impl<R: Read + Seek> IndexSource for MappedReader<'_, R> {
    fn resolve_list(
        &mut self,
        attr: &[u8],
        geometry: Geometry,
        progress: &Progress,
    ) -> Result<Vec<ListEntry>, String> {
        read_attribute_list_buffered(
            self.file,
            attr,
            geometry,
            progress,
            self.cursor,
            &mut self.list_buffer,
        )
    }
    fn validate_record(&mut self, index: usize, raw: &[u8]) -> Result<(), String> {
        if let Some(bootstrap) = &mut self.bootstrap
            && !bootstrap.records.is_empty()
            && let Some(expected) = bootstrap.records.get(&index)
        {
            if *expected != BootstrapRecord::from_raw(index, raw)? {
                return Err(
                    "$MFT attributes or identity changed since bootstrap; rescan the live volume"
                        .into(),
                );
            }
            bootstrap.records.remove(&index);
        }
        Ok(())
    }
    fn validate_parsed(&self, record: &format::Record) -> Result<(), String> {
        if let Some(bootstrap) = &self.bootstrap
            && record.reference == bootstrap.first_reference
            && record.required != bootstrap.required
        {
            return Err(
                "$MFT attribute list changed since bootstrap; rescan the live volume".into(),
            );
        }
        Ok(())
    }
    fn finish_validation(&self) -> Result<(), String> {
        if self
            .bootstrap
            .as_ref()
            .is_some_and(|bootstrap| !bootstrap.records.is_empty())
        {
            return Err(
                "Missing bulk $MFT record used during bootstrap; rescan the live volume".into(),
            );
        }
        Ok(())
    }
}

/// Resolve a complete unnamed list without changing the sequential MFT cursor.
/// Reads are bounded, sector aligned, cancellation aware and volume bounded.
pub(super) fn read_attribute_list<R: Read + Seek>(
    file: &mut R,
    attr: &[u8],
    geometry: Geometry,
    progress: &Progress,
) -> Result<Vec<ListEntry>, String> {
    read_attribute_list_buffered(file, attr, geometry, progress, None, &mut Vec::new())
}

fn read_attribute_list_buffered<R: Read + Seek>(
    file: &mut R,
    attr: &[u8],
    geometry: Geometry,
    progress: &Progress,
    saved_cursor: Option<u64>,
    bytes: &mut Vec<u8>,
) -> Result<Vec<ListEntry>, String> {
    if progress.cancelled() {
        return Err("NTFS metadata read cancelled".into());
    }
    if !format::attribute_name(attr)?.is_empty() {
        return Err("Unsupported named NTFS attribute list".into());
    }
    if attr.get(8) == Some(&0) {
        return format::attribute_list(format::resident(attr)?);
    }
    geometry.validate()?;
    if geometry.cluster < geometry.sector as u64
        || !geometry.cluster.is_power_of_two()
        || geometry.cluster > 2 * 1024 * 1024
    {
        return Err("Unsupported attribute-list cluster geometry".into());
    }
    let length = format::u64_at(attr, 48).ok_or("Missing attribute-list length")?;
    if length == 0 || length > MAX_LIST_BYTES {
        return Err("NTFS attribute list exceeds the supported safety bound or is empty".into());
    }
    let allocation = format::u64_at(attr, 40).ok_or("Missing attribute-list allocation")?;
    let initialized =
        format::u64_at(attr, 56).ok_or("Missing attribute-list initialized length")?;
    if length > allocation
        || initialized < length
        || initialized > allocation
        || !allocation.is_multiple_of(geometry.cluster)
    {
        return Err("Incomplete or inconsistent NTFS attribute-list lengths".into());
    }
    let segment = segment(attr)?;
    if segment.vcn != 0 {
        return Err("Unsupported split NTFS attribute-list mapping; the complete list mapping must be in its base attribute".into());
    }
    if segment
        .highest
        .checked_add(1)
        .and_then(|n| n.checked_mul(geometry.cluster))
        != Some(allocation)
    {
        return Err("Incomplete NTFS attribute-list allocation coverage".into());
    }
    let aligned = length
        .checked_next_multiple_of(geometry.sector as u64)
        .ok_or("Attribute-list length overflow")?;
    let mapping = assemble(
        vec![segment],
        geometry.cluster,
        aligned,
        geometry
            .total_clusters
            .ok_or("Missing volume bounds for external NTFS metadata")?,
    )?;
    let original = match saved_cursor {
        Some(position) => position,
        None => file
            .stream_position()
            .map_err(|e| format!("Saving NTFS read cursor: {e}"))?,
    };
    let result = (|| {
        let mut reader = MappedReader::new(file, mapping, geometry.cluster, aligned)?;
        let needed = usize::try_from(aligned).map_err(|_| "Attribute-list allocation overflow")?;
        if bytes.capacity() < needed {
            // Reuse storage, not metadata: every requested byte is reread below.
            // Exact reservation avoids geometric growth beyond the 16 MiB bound.
            bytes
                .try_reserve_exact(needed.saturating_sub(bytes.len()))
                .map_err(|_| "Insufficient memory for the NTFS attribute list")?;
        }
        bytes.resize(needed, 0);
        for chunk in bytes.chunks_mut(64 * 1024) {
            if progress.cancelled() {
                return Err("NTFS metadata read cancelled".into());
            }
            reader.read_exact(chunk).map_err(|e| {
                format!("Reading NTFS attribute list ({length} logical bytes): {e}")
            })?;
        }
        if progress.cancelled() {
            return Err("NTFS metadata read cancelled".into());
        }
        let entries = format::attribute_list(&bytes[..length as usize])?;
        if entries.is_empty() {
            return Err(
                "Empty nonresident NTFS attribute list; completeness cannot be verified".into(),
            );
        }
        Ok(entries)
    })();
    // Restore even after cancellation, truncation or parsing failure.
    file.seek(SeekFrom::Start(original))
        .map_err(|e| format!("Restoring NTFS bulk read cursor: {e}"))?;
    result
}
impl<R: Read + Seek> Read for MappedReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.remaining == 0 {
            return Ok(0);
        }
        let run = self
            .runs
            .get(self.run)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "Missing NTFS extent"))?;
        let length = run.clusters * self.cluster;
        let pos = run
            .lcn
            .unwrap()
            .checked_mul(self.cluster)
            .and_then(|p| p.checked_add(self.in_run))
            .ok_or_else(|| io::Error::other("NTFS offset overflow"))?;
        let len = (buf.len() as u64)
            .min(length - self.in_run)
            .min(self.remaining) as usize;
        // Seek only at extent boundaries; the read cursor is otherwise sequential.
        if self.in_run == 0 {
            self.file.seek(SeekFrom::Start(pos))?;
        }
        let read = self.file.read(&mut buf[..len])?;
        self.cursor = pos.checked_add(read as u64);
        self.in_run += read as u64;
        self.remaining -= read as u64;
        if self.in_run == length {
            self.run += 1;
            self.in_run = 0;
        }
        Ok(read)
    }
}

pub(super) fn saved_geometry(file: &mut File) -> Result<Geometry, String> {
    let mut header = [0u8; 48];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if header[..4] != *b"FILE" {
        return Err("Saved MFT must start with record zero (FILE)".into());
    }
    let size = format::u32_at(&header, 28).ok_or("Missing record size")? as usize;
    let count = format::u16_at(&header, 6).ok_or("Missing sector sequence")? as usize;
    if count < 2 || !size.is_multiple_of(count - 1) {
        return Err("Invalid saved NTFS geometry".into());
    }
    let mut geometry = Geometry {
        record_size: size,
        sector: size / (count - 1),
        cluster: 0,
        valid: file.metadata().map_err(|e| e.to_string())?.len(),
        total_clusters: None,
    };
    geometry.validate()?;
    let mut raw = vec![0; size];
    file.seek(SeekFrom::Start(3 * size as u64))
        .map_err(|e| e.to_string())?;
    file.read_exact(&mut raw)
        .map_err(|e| format!("Missing $Volume record: {e}"))?;
    format::fixup(&mut raw, geometry.sector, false)?;
    let attrs = format::attributes(&raw)?;
    let info = attrs
        .into_iter()
        .find(|a| format::u32_at(a, 0) == Some(0x70))
        .ok_or("Missing NTFS volume version")?;
    let info = format::resident(info)?;
    check_version(
        *info.get(8).ok_or("Missing NTFS major version")? as u16,
        *info.get(9).ok_or("Missing NTFS minor version")? as u16,
    )?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    file.read_exact(&mut raw).map_err(|e| e.to_string())?;
    format::fixup(&mut raw, geometry.sector, false)?;
    let first_attrs = format::attributes(&raw)?;
    for list in first_attrs
        .iter()
        .filter(|a| format::u32_at(a, 0) == Some(0x20))
    {
        if list[8] != 0 {
            return Err("Flat saved MFT cannot resolve a nonresident record-zero attribute list: external volume clusters are unavailable; use a live NTFS scan.".into());
        }
        if format::attribute_list(format::resident(list)?)?
            .iter()
            .any(|e| e.typ == 0x80 && e.name.is_empty() && e.vcn != 0)
        {
            return Err("Saved-index replay needs a complete record-zero MFT mapping to infer cluster size; live fragmented-volume reading remains supported.".into());
        }
    }
    let data = first_attrs
        .into_iter()
        .find(|a| {
            format::u32_at(a, 0) == Some(0x80)
                && a[8] == 1
                && a[9] == 0
                && format::u64_at(a, 16) == Some(0)
        })
        .ok_or("Missing saved $MFT data geometry")?;
    let extent = segment(data)?;
    let clusters = extent
        .highest
        .checked_add(1)
        .ok_or("Saved cluster count overflow")?;
    let allocation = format::u64_at(data, 40).ok_or("Missing saved $MFT allocation")?;
    if clusters == 0 || !allocation.is_multiple_of(clusters) {
        return Err("Invalid saved cluster geometry".into());
    }
    geometry.cluster = allocation / clusters;
    if geometry.cluster < geometry.sector as u64
        || !geometry.cluster.is_power_of_two()
        || geometry.cluster > 2 * 1024 * 1024
    {
        return Err("Unsupported saved NTFS cluster size".into());
    }
    Ok(geometry)
}
fn check_version(major: u16, minor: u16) -> Result<(), String> {
    if major != 3 || minor > 1 {
        return Err(format!("Unsupported NTFS version {major}.{minor}"));
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct Segment {
    pub vcn: u64,
    pub highest: u64,
    pub runs: Vec<Run>,
}
pub(super) fn segment(attr: &[u8]) -> Result<Segment, String> {
    if attr.get(8) != Some(&1) || format::u16_at(attr, 12) != Some(0) {
        return Err("Unsupported resident/compressed/sparse NTFS metadata mapping".into());
    }
    let vcn = format::u64_at(attr, 16).ok_or("Missing mapping VCN")?;
    let highest = format::u64_at(attr, 24).ok_or("Missing mapping end")?;
    let offset = format::u16_at(attr, 32).ok_or("Missing mapping offset")? as usize;
    if offset < 64 {
        return Err("Mapping pairs overlap attribute header".into());
    }
    let runs = format::runs(attr.get(offset..).ok_or("Invalid mapping offset")?)?;
    let clusters = runs
        .iter()
        .try_fold(0u64, |sum, r| sum.checked_add(r.clusters))
        .ok_or("Mapping VCN overflow")?;
    if highest.checked_sub(vcn).and_then(|n| n.checked_add(1)) != Some(clusters) {
        return Err("NTFS mapping VCN range disagrees with its runs".into());
    }
    Ok(Segment { vcn, highest, runs })
}
pub(super) fn assemble(
    mut segments: Vec<Segment>,
    cluster: u64,
    valid: u64,
    total_clusters: u64,
) -> Result<Vec<Run>, String> {
    segments.sort_unstable_by_key(|s| s.vcn);
    let mut next = 0;
    let mut result = vec![];
    for seg in segments {
        if seg.vcn != next {
            return Err("Incomplete or overlapping NTFS extent segments".into());
        }
        next = seg.highest.checked_add(1).ok_or("NTFS VCN overflow")?;
        for run in seg.runs {
            let end = run
                .lcn
                .and_then(|lcn| lcn.checked_add(run.clusters))
                .ok_or("Sparse or overflowing NTFS metadata extent")?;
            if end > total_clusters {
                return Err("NTFS metadata extent exceeds the volume".into());
            }
            end.checked_mul(cluster)
                .ok_or("NTFS metadata offset overflow")?;
            result.push(run);
        }
    }
    if cluster == 0 || next.checked_mul(cluster).filter(|&n| n >= valid).is_none() {
        return Err("Incomplete NTFS extent coverage".into());
    }
    Ok(result)
}

#[cfg(windows)]
pub(super) fn volume_mapping(
    file: &mut File,
    progress: &Progress,
) -> Result<(Geometry, Bootstrap), String> {
    use crate::scanner::windows;
    let (data, version) =
        windows::ntfs_data(file).map_err(|e| format!("Not an accessible NTFS volume: {e}"))?;
    check_version(version.0, version.1)?;
    if data.MftValidDataLength <= 0
        || data.MftStartLcn < 0
        || data.TotalClusters <= 0
        || data.BytesPerCluster == 0
    {
        return Err("Unsupported NTFS volume geometry".into());
    }
    let geometry = Geometry {
        record_size: data.BytesPerFileRecordSegment as usize,
        sector: data.BytesPerSector as usize,
        cluster: data.BytesPerCluster as u64,
        valid: data.MftValidDataLength as u64,
        total_clusters: Some(data.TotalClusters as u64),
    };
    geometry.validate()?;
    let total = data.TotalClusters as u64;
    let offset = (data.MftStartLcn as u64)
        .checked_mul(geometry.cluster)
        .ok_or("MFT offset overflow")?;
    let mut first = vec![0; geometry.record_size];
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Seeking $MFT record zero (offset {offset}): {e}"))?;
    file.read_exact(&mut first).map_err(|e| {
        format!(
            "Reading $MFT record zero ({} bytes, {}-byte sectors): {e}",
            geometry.record_size, geometry.sector
        )
    })?;
    format::fixup(&mut first, geometry.sector, false)?;
    let mapping = mapping_from_record(
        file,
        &first,
        geometry,
        total,
        progress,
        |file, reference| {
            windows::ntfs_record(file, reference, geometry.record_size).map_err(|e| e.to_string())
        },
    )?;
    Ok((geometry, mapping))
}

pub(super) fn mapping_from_record<R: Read + Seek>(
    file: &mut R,
    first: &[u8],
    geometry: Geometry,
    total: u64,
    progress: &Progress,
    mut load_extension: impl FnMut(&mut R, u64) -> Result<Vec<u8>, String>,
) -> Result<Bootstrap, String> {
    if progress.cancelled() {
        return Err("NTFS bootstrap cancelled".into());
    }
    let first_ref = (format::u16_at(first, 16).ok_or("Missing $MFT sequence")? as u64) << 48;
    let attrs = format::attributes(first)?;
    let first_record = BootstrapRecord::from_raw(0, first)?;
    if first_record.base != 0 || first_record.flags & 2 != 0 {
        return Err("Invalid $MFT base record".into());
    }
    let mut records = HashMap::from([(0, first_record)]);
    let mut cached = HashMap::new();
    let mut required = Vec::new();
    let mut segments = vec![];
    for attr in &attrs {
        if format::u32_at(attr, 0) == Some(0x80) && format::attribute_name(attr)?.is_empty() {
            segments.push(segment(attr)?);
        }
    }
    let lists: Vec<_> = attrs
        .iter()
        .filter(|a| format::u32_at(a, 0) == Some(0x20))
        .collect();
    if lists.len() > 1 {
        return Err("Unsupported multiple $MFT attribute-list mappings".into());
    }
    for attr in lists {
        let mut list_geometry = geometry;
        list_geometry.total_clusters = Some(total);
        required.extend(read_attribute_list(file, attr, list_geometry, progress)?);
    }
    let mut seen = HashSet::new();
    for entry in &required {
        if !seen.insert(entry) {
            return Err("Duplicate listed $MFT attribute".into());
        }
        if entry.typ == 0x80 && entry.name.is_empty() {
            if progress.cancelled() {
                return Err("NTFS bootstrap cancelled".into());
            }
            if entry.reference == first_ref {
                if !attrs.iter().any(|a| {
                    format::u32_at(a, 0) == Some(entry.typ)
                        && format::u16_at(a, 14) == Some(entry.instance)
                        && format::u64_at(a, 16) == Some(entry.vcn)
                        && a[9] == 0
                }) {
                    return Err("$MFT base mapping disagrees with its attribute list".into());
                }
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(cached_entry) =
                cached.entry(entry.reference)
            {
                if records
                    .len()
                    .checked_add(1)
                    .and_then(|n| n.checked_mul(geometry.record_size))
                    .is_none_or(|n| n > MAX_BOOTSTRAP_BYTES)
                {
                    return Err("$MFT bootstrap metadata exceeds the supported safety bound".into());
                }
                let mut raw = load_extension(file, entry.reference)
                    .map_err(|e| format!("Cannot read $MFT mapping extension: {e}"))?;
                format::fixup(&mut raw, geometry.sector, true)?;
                if format::u16_at(&raw, 16) != Some((entry.reference >> 48) as u16)
                    || format::u64_at(&raw, 32) != Some(first_ref)
                    || format::u16_at(&raw, 22).is_none_or(|flags| flags & 1 == 0)
                {
                    return Err("$MFT extension changed during bootstrap".into());
                }
                records.insert(
                    format::slot(entry.reference),
                    BootstrapRecord::from_raw(format::slot(entry.reference), &raw)?,
                );
                cached_entry.insert(raw);
            }
            let raw = &cached[&entry.reference];
            let attr = format::attributes(raw)?
                .into_iter()
                .find(|a| {
                    format::u32_at(a, 0) == Some(entry.typ)
                        && format::u16_at(a, 14) == Some(entry.instance)
                        && format::u64_at(a, 16) == Some(entry.vcn)
                        && a[9] == 0
                })
                .ok_or("Missing listed $MFT extent attribute")?;
            segments.push(segment(attr)?);
        }
    }
    let runs = assemble(segments, geometry.cluster, geometry.valid, total)?;
    Ok(Bootstrap {
        runs,
        records,
        required,
        first_reference: first_ref,
    })
}
