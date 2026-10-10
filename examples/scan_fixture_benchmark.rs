//! Read-only bulk-parser timing on a supplied generated/sanitized flat MFT.
use canopy::scanner::{Progress, mft};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

fn main() -> Result<(), String> {
    let input = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("Supply a generated flat MFT fixture")?;
    let bytes = std::fs::metadata(&input).map_err(|e| e.to_string())?.len();
    let mut timings = vec![];
    for _ in 0..5 {
        let started = Instant::now();
        let scan = mft::scan_file(Path::new("Z:\\"), &input, &Progress::default())?;
        timings.push(serde_json::json!({"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,
            "read_ms":scan.stats.mft_read_seconds*1000.0,"tree_ms":scan.stats.tree_build_seconds*1000.0,
            "nodes":scan.nodes.len(),"allocated":scan.nodes[0].allocated,"metadata_queries":scan.stats.file_metadata_queries}));
    }
    println!(
        "{}",
        serde_json::json!({"input_bytes":bytes,"node_bytes":std::mem::size_of::<canopy::model::Node>(),"runs":timings})
    );
    Ok(())
}
