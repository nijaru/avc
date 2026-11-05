//! Reproducible pure-engine cost probe, not a storage/competitor benchmark.
use avc::engine::{Edit, Engine, Limits};
use avc::{Entry, Result};
use std::{collections::BTreeSet, time::Instant};
fn main() -> Result<()> {
    let width = std::env::args()
        .nth(1)
        .map(|n| n.parse::<usize>().expect("integer width"))
        .unwrap_or(10_000);
    let shared = std::env::args().nth(2).is_some_and(|s| s == "shared");
    let prefix = if shared {
        "x".repeat(220)
    } else {
        String::new()
    };
    let mut engine = Engine::new();
    let mut root = engine.empty()?;
    let blob = engine.blob(b"source\n".to_vec())?;
    let file = Entry::File {
        blob,
        executable: false,
    };
    let build = Instant::now();
    for i in 0..width {
        root = engine.edit(
            root,
            &[Edit {
                path: vec![format!("{prefix}file-{i:07}").into_bytes()],
                expected: Entry::Absent,
                value: file.clone(),
            }],
        )?;
    }
    let build_time = build.elapsed();
    engine.verify(root, Limits::default())?;
    let before: BTreeSet<_> = engine.objects().map(|(id, _)| id).collect();
    let old = root;
    let replacement = Entry::File {
        blob: engine.blob(b"changed\n".to_vec())?,
        executable: false,
    };
    let key = format!("{prefix}file-{:07}", width / 2).into_bytes();
    let start = Instant::now();
    root = engine.edit(
        root,
        &[Edit {
            path: vec![key.clone()],
            expected: file,
            value: replacement,
        }],
    )?;
    let edit_time = start.elapsed();
    let start = Instant::now();
    let diff = engine.diff(old, root)?;
    let diff_time = start.elapsed();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, vec![key]);
    let mut nodes = 0;
    let mut bytes = 0;
    for (id, object) in engine.objects() {
        if !before.contains(&id) {
            nodes += 1;
            bytes += object.encode()?.len();
        }
    }
    let start = Instant::now();
    engine.verify(root, Limits::default())?;
    let verify_time = start.elapsed();
    println!(
        "width={width} shared_prefix={shared} build_ms={:.3} edit_us={:.3} diff_us={:.3} verify_ms={:.3} new_objects={nodes} new_canonical_bytes={bytes}",
        build_time.as_secs_f64() * 1000.,
        edit_time.as_secs_f64() * 1e6,
        diff_time.as_secs_f64() * 1e6,
        verify_time.as_secs_f64() * 1000.
    );
    Ok(())
}
