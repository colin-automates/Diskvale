//! The same predicate drives file search and filtered map totals.
use crate::{
    file_types::Category,
    model::{Metric, Scan},
};

const DAY: u64 = 86_400;

#[derive(Default, Clone)]
pub struct Query {
    terms: Vec<String>,
    extension: Option<String>,
    minimum: Option<u64>,
    maximum: Option<u64>,
    /// Modified strictly before this Unix time (`older:`).
    modified_before: Option<u64>,
    /// Modified strictly after this Unix time (`newer:`).
    modified_after: Option<u64>,
    pub error: Option<String>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        Self::parse_at(input, now)
    }
    /// `now` is the Unix time that `older:` and `newer:` ages count back from.
    pub fn parse_at(input: &str, now: u64) -> Self {
        let mut result = Self::default();
        for token in input.split_whitespace() {
            let token = token.to_ascii_lowercase();
            if let Some(age) = token
                .strip_prefix("older:")
                .or_else(|| token.strip_prefix("newer:"))
            {
                match parse_age(age) {
                    Some(seconds) if token.starts_with("older:") => {
                        result.modified_before = Some(now.saturating_sub(seconds))
                    }
                    Some(seconds) => result.modified_after = Some(now.saturating_sub(seconds)),
                    None => {
                        result.error = Some(
                            "Use an age such as older:1y, older:6m, newer:30d or newer:2w".into(),
                        )
                    }
                }
            } else if let Some(ext) = token.strip_prefix("ext:") {
                if ext.is_empty() {
                    result.error = Some("Add an extension after ext:, e.g. ext:mp4".into());
                }
                result.extension = Some(ext.trim_start_matches('.').into());
            } else if token.starts_with(['>', '<']) {
                match parse_size(&token[1..]) {
                    Some(size) if token.starts_with('>') => result.minimum = Some(size),
                    Some(size) => result.maximum = Some(size),
                    None => result.error = Some("Use a size such as >1GB, >500MiB or <20MB".into()),
                }
            } else {
                result.terms.push(token);
            }
        }
        result
    }
    /// `modified` is Unix seconds, or 0 when the scan did not record it. An
    /// unknown date never satisfies an age filter, since it cannot be proven.
    pub fn matches(&self, name: &str, size: u64, modified: u64) -> bool {
        if self.error.is_some()
            || self.minimum.is_some_and(|v| size <= v)
            || self.maximum.is_some_and(|v| size >= v)
            || self
                .modified_before
                .is_some_and(|v| modified == 0 || modified >= v)
            || self
                .modified_after
                .is_some_and(|v| modified == 0 || modified <= v)
        {
            return false;
        }
        let name = name.to_lowercase();
        if self.extension.as_ref().is_some_and(|ext| {
            std::path::Path::new(&name)
                .extension()
                .and_then(|s| s.to_str())
                != Some(ext.as_str())
        }) {
            return false;
        }
        self.terms.iter().all(|term| name.contains(term))
    }
}

/// Ages are a whole number plus `d`ays, `w`eeks, `m`onths (30 days) or `y`ears (365 days).
fn parse_age(input: &str) -> Option<u64> {
    let unit = input.chars().last()?;
    let days = match unit {
        'd' => 1,
        'w' => 7,
        'm' => 30,
        'y' => 365,
        _ => return None,
    };
    let count: u64 = input[..input.len() - 1].parse().ok()?;
    count.checked_mul(days)?.checked_mul(DAY)
}

fn parse_size(input: &str) -> Option<u64> {
    let split = input
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(input.len());
    let number: f64 = input[..split].parse().ok()?;
    let scale = match &input[split..] {
        "" | "b" => 1.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1_048_576.0,
        "gib" => 1_073_741_824.0,
        "tib" => 1_099_511_627_776.0,
        _ => return None,
    };
    let value = number * scale;
    (value.is_finite() && value >= 0.0 && value < u64::MAX as f64).then_some(value as u64)
}

pub fn weights(scan: &Scan, metric: Metric, query: &Query, category: Option<Category>) -> Vec<u64> {
    map_totals(scan, metric, query, category).bytes
}

/// Bytes and matching file counts are accumulated in the same background pass.
pub struct MapTotals {
    pub bytes: Vec<u64>,
    pub files: Vec<u64>,
}

pub fn map_totals(
    scan: &Scan,
    metric: Metric,
    query: &Query,
    category: Option<Category>,
) -> MapTotals {
    let mut result = vec![0u64; scan.nodes.len()];
    let mut files = vec![0u64; scan.nodes.len()];
    let mut order = Vec::with_capacity(scan.nodes.len());
    let mut stack = vec![0];
    while let Some(id) = stack.pop() {
        order.push(id);
        let node = &scan.nodes[id];
        stack.extend(node.children.iter().copied());
        if !node.directory
            && query.matches(&node.name, node.size(metric), node.modified)
            && category.is_none_or(|c| Category::from_name(&node.name) == c)
        {
            result[id] = node.size(metric);
            files[id] = 1;
        }
    }
    for id in order.into_iter().rev() {
        if let Some(parent) = scan.nodes[id].parent {
            result[parent] = result[parent].saturating_add(result[id]);
            files[parent] = files[parent].saturating_add(files[id]);
        }
    }
    MapTotals {
        bytes: result,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Node, ScanStats};
    #[test]
    fn structured_search_uses_explicit_decimal_and_binary_units() {
        let query = Query::parse("ext:MP4 >1GB <2GiB capture");
        assert!(query.matches("Capture.MP4", 1_010_000_000, 0));
        assert!(!query.matches("capture.mp4", 1_000_000_000, 0));
        assert!(!query.matches("capture.zip", 1_010_000_000, 0));
        assert!(!query.matches("other.mp4", 1_010_000_000, 0));
        assert!(!Query::parse(">bad").matches("a", 50, 0));
    }
    #[test]
    fn age_filters_count_back_from_now_and_never_match_unknown_dates() {
        let now = 1_790_812_800;
        let year = 365 * DAY;
        let old = Query::parse_at("older:1y ext:zip", now);
        assert!(old.matches("a.zip", 1, now - year - 1));
        assert!(
            !old.matches("a.zip", 1, now - year + 1),
            "newer than a year"
        );
        assert!(
            !old.matches("a.zip", 1, now - year),
            "boundary is exclusive"
        );
        assert!(
            !old.matches("a.zip", 1, 0),
            "unknown date cannot be proven old"
        );
        assert!(!old.matches("a.rar", 1, now - 2 * year));
        let recent = Query::parse_at("newer:30d", now);
        assert!(recent.matches("a", 1, now - 29 * DAY));
        assert!(!recent.matches("a", 1, now - 31 * DAY));
        assert!(!recent.matches("a", 1, 0));
        let window = Query::parse_at("older:1w newer:6m", now);
        assert!(window.matches("a", 1, now - 30 * DAY));
        assert!(!window.matches("a", 1, now - 3 * DAY));
        assert!(!window.matches("a", 1, now - 200 * DAY));
        for bad in [
            "older:",
            "older:y",
            "older:1",
            "older:1x",
            "newer:-3d",
            "older:1.5y",
        ] {
            let query = Query::parse_at(bad, now);
            assert!(query.error.is_some(), "{bad}");
            assert!(!query.matches("a", 1, now - 5 * year));
        }
    }
    #[test]
    fn filtering_reaggregates_arbitrarily_ordered_ancestors_without_changing_source() {
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: ScanStats::default(),
            nodes: vec![
                Node::new("C:".into(), None, true),
                Node::new("clip.mp4".into(), Some(2), false),
                Node::new("folder".into(), Some(0), true),
                Node::new("other.zip".into(), Some(0), false),
            ],
        };
        scan.nodes[0].children = vec![2, 3];
        scan.nodes[2].children = vec![1];
        scan.nodes[1].allocated = 1200;
        scan.nodes[3].allocated = 800;
        scan.finish();
        assert_eq!(
            weights(
                &scan,
                Metric::Allocated,
                &Query::parse(">1KB"),
                Some(Category::Media)
            ),
            vec![1200, 1200, 1200, 0]
        );
        assert_eq!(scan.nodes[0].allocated, 2000);
        let totals = map_totals(&scan, Metric::Allocated, &Query::parse("ext:mp4"), None);
        assert_eq!(totals.bytes, vec![1200, 1200, 1200, 0]);
        assert_eq!(
            totals.files,
            vec![1, 1, 1, 0],
            "Matching counts included the excluded zip file"
        );
        let predicate = Query::parse("ext:mp4 >1KB");
        assert_eq!(
            scan.extensions_matching(0, Metric::Allocated, |n| predicate.matches(
                &n.name,
                n.allocated,
                n.modified
            )),
            vec![(".mp4".into(), 1200, 1)]
        );
    }
}
