use super::format::{self, Run};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

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
        })
    }
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
        if list[8] != 0
            || format::attribute_list(format::resident(list)?)?
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
            result.push(run);
        }
    }
    if cluster == 0 || next.checked_mul(cluster).filter(|&n| n >= valid).is_none() {
        return Err("Incomplete NTFS extent coverage".into());
    }
    Ok(result)
}

#[cfg(windows)]
pub(super) fn volume_mapping(file: &mut File) -> Result<(Geometry, Vec<Run>), String> {
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
    let mapping = mapping_from_record(file, &first, geometry, total, |file, reference| {
        windows::ntfs_record(file, reference, geometry.record_size).map_err(|e| e.to_string())
    })?;
    Ok((geometry, mapping))
}

pub(super) fn mapping_from_record<R: Read + Seek>(
    file: &mut R,
    first: &[u8],
    geometry: Geometry,
    total: u64,
    mut load_extension: impl FnMut(&mut R, u64) -> Result<Vec<u8>, String>,
) -> Result<Vec<Run>, String> {
    let first_ref = (format::u16_at(first, 16).ok_or("Missing $MFT sequence")? as u64) << 48;
    let attrs = format::attributes(first)?;
    let mut segments = vec![];
    for attr in &attrs {
        if format::u32_at(attr, 0) == Some(0x80) && format::attribute_name(attr)?.is_empty() {
            segments.push(segment(attr)?);
        }
    }
    for attr in attrs.iter().filter(|a| format::u32_at(a, 0) == Some(0x20)) {
        let list = if attr[8] == 0 {
            format::resident(attr)?.to_vec()
        } else {
            let length = format::u64_at(attr, 48).ok_or("Missing attribute-list length")?;
            if length > 16 * 1024 * 1024 {
                return Err("NTFS attribute list exceeds the supported safety bound".into());
            }
            // Volume reads must use whole sectors, even when an attribute list's
            // logical length ends mid-sector. Padding stays within its mapped extent.
            let aligned = length
                .checked_next_multiple_of(geometry.sector as u64)
                .ok_or("Attribute-list length overflow")?;
            let mapping = assemble(vec![segment(attr)?], geometry.cluster, aligned, total)?;
            let mut reader = MappedReader::new(file, mapping, geometry.cluster, aligned)?;
            let mut bytes = vec![0; aligned as usize];
            reader.read_exact(&mut bytes).map_err(|e| format!("Reading $MFT attribute list ({length} logical bytes, {aligned} sector-aligned bytes): {e}"))?;
            bytes.truncate(length as usize);
            bytes
        };
        let mut cached = std::collections::HashMap::new();
        for entry in format::attribute_list(&list)?
            .into_iter()
            .filter(|e| e.typ == 0x80 && e.name.is_empty())
        {
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
                let mut raw = load_extension(file, entry.reference)
                    .map_err(|e| format!("Cannot read $MFT mapping extension: {e}"))?;
                format::fixup(&mut raw, geometry.sector, true)?;
                if format::u16_at(&raw, 16) != Some((entry.reference >> 48) as u16)
                    || format::u64_at(&raw, 32) != Some(first_ref)
                    || format::u16_at(&raw, 22).is_none_or(|flags| flags & 1 == 0)
                {
                    return Err("$MFT extension changed during bootstrap".into());
                }
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
    assemble(segments, geometry.cluster, geometry.valid, total)
}
