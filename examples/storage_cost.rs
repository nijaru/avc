//! Reproducible equivalent-work repository storage cost probe, not a competitor
//! benchmark. One process run = one repetition on a fresh repository; interleave
//! variant runs at the runner level for matched conditions.
//!
//! Usage: `storage_cost <small-churn|binary-churn> <rep> [label]`
//!
//! Runs must set `AVC_STORAGE_COST_REV` to the source revision being measured
//! (e.g. `git rev-parse HEAD`); results record it so guard/fence behavior can
//! be attributed and runs without it are not evidence.
//!
//! Measures acknowledged commit cost (commit_source round trip), read cost
//! (full verified load after every acknowledged commit), export cost (complete
//! object-only stream), import cost into a fresh store, reopen cost, and
//! on-disk bytes/file counts. All workload bytes are deterministic and identical
//! across runs and storage variants (recorded as a content fingerprint).
//!
//! Durability contract is identical in every variant: the repository refuses to
//! open unless SQLite reports WAL plus synchronous=FULL (and macOS fullfsync),
//! so every acknowledged commit measured here paid that contract.
use avc::engine::{Edit, Engine};
use avc::repository::Repository;
use avc::{Entry, Object};
use std::{io::Cursor, path::Path, time::Instant};

/// Deterministic xorshift64 content; identical bytes in every run/variant.
fn content(seed: u64, len: usize) -> Vec<u8> {
    let mut x = (seed ^ 0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

struct Workload {
    name: &'static str,
    file_count: usize,
    file_bytes: Vec<usize>,
    revisions: usize,
    replaced_per_rev: usize,
}

fn workload(name: &str) -> Workload {
    match name {
        // Many small source-like files, repeated partial replacement churn.
        "small-churn" => Workload {
            name: "small-churn",
            file_count: 200,
            file_bytes: (0..200)
                .map(|i| [64, 256, 1024, 2048, 4096][i % 5])
                .collect(),
            revisions: 30,
            replaced_per_rev: 20,
        },
        // Fewer larger binary assets under the bounded 16 MiB payload /
        // 256 MiB closure model.
        "binary-churn" => Workload {
            name: "binary-churn",
            file_count: 8,
            file_bytes: vec![4 * 1024 * 1024; 8],
            revisions: 10,
            replaced_per_rev: 2,
        },
        other => panic!("unknown workload: {other}"),
    }
}

fn file_name(i: usize) -> Vec<u8> {
    format!("asset-{i:04}").into_bytes()
}

fn summary(ms: &[f64]) -> serde_json::Value {
    let mut sorted = ms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| -> f64 {
        let n = sorted.len();
        sorted[(p * n as f64).ceil().max(1.0) as usize - 1]
    };
    let total: f64 = ms.iter().sum();
    serde_json::json!({
        "n": ms.len(),
        "total_ms": total,
        "mean_ms": total / ms.len() as f64,
        "p50_ms": pct(0.50),
        "p95_ms": pct(0.95),
        "max_ms": sorted[sorted.len() - 1],
    })
}

/// (bytes, files) under a directory tree.
fn disk_usage(dir: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            let (b, f) = disk_usage(&path);
            bytes += b;
            files += f;
        } else {
            bytes += entry.metadata().unwrap().len();
            files += 1;
        }
    }
    (bytes, files)
}

fn named_file_size(dir: &Path, name: &str) -> u64 {
    std::fs::metadata(dir.join(name))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let spec = workload(&args.next().expect("workload name"));
    let rep: usize = args
        .next()
        .expect("rep number")
        .parse()
        .expect("integer rep");
    let label = args.next().unwrap_or_else(|| "unlabeled".to_owned());
    let source_rev = std::env::var("AVC_STORAGE_COST_REV")
        .expect("set AVC_STORAGE_COST_REV to the measured source revision");

    let w = &spec;
    let mut fingerprint = blake3::Hasher::new();
    let mut engine = Engine::new();
    let mut root = engine.empty().unwrap();

    let build = Instant::now();
    for i in 0..w.file_count {
        let bytes = content(i as u64 * 2 + 1, w.file_bytes[i]);
        fingerprint.update(&bytes);
        let blob = engine.blob(bytes).unwrap();
        root = engine
            .edit(
                root,
                &[Edit {
                    path: vec![file_name(i)],
                    expected: Entry::Absent,
                    value: Entry::File {
                        blob,
                        executable: false,
                    },
                }],
            )
            .unwrap();
    }
    let initial_engine_ms = build.elapsed().as_secs_f64() * 1e3;

    let project = tempfile::tempdir().unwrap();
    let restore_dir = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();

    let mut commit_ms = Vec::new();
    let mut read_ms = Vec::new();
    let mut engine_ms = Vec::new();
    let mut last_changed = 0usize;
    for rev in 0..w.revisions {
        let churn = Instant::now();
        let mut edits = Vec::new();
        for j in 0..w.replaced_per_rev {
            let i = (rev * w.replaced_per_rev + j) % w.file_count;
            last_changed = i;
            let bytes = content(
                i as u64 * 2 + 1 + (rev as u64 + 1) * 1_000_003,
                w.file_bytes[i],
            );
            fingerprint.update(&bytes);
            let blob = engine.blob(bytes).unwrap();
            let path = vec![file_name(i)];
            let expected = engine.read(root, &path).unwrap();
            edits.push(Edit {
                path,
                expected,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            });
        }
        root = engine.edit(root, &edits).unwrap();
        engine_ms.push(churn.elapsed().as_secs_f64() * 1e3);

        // Acknowledged commit: the API returns only after the durable receipt.
        let start = Instant::now();
        repository
            .commit_source(&engine, root, rev as u64, &format!("rep-{rep}-rev-{rev}"))
            .unwrap();
        commit_ms.push(start.elapsed().as_secs_f64() * 1e3);

        // Read: full verified load plus an exact byte check of a changed file.
        let start = Instant::now();
        let (selected, loaded) = repository.load().unwrap();
        assert_eq!(selected.root, root);
        let entry = loaded.read(root, &[file_name(last_changed)]).unwrap();
        let Entry::File { blob, .. } = entry else {
            panic!("expected file entry");
        };
        let Object::Blob(got) = loaded.object(blob).unwrap() else {
            panic!("expected blob object");
        };
        assert_eq!(
            got,
            &content(
                last_changed as u64 * 2 + 1 + (rev as u64 + 1) * 1_000_003,
                w.file_bytes[last_changed]
            )
        );
        read_ms.push(start.elapsed().as_secs_f64() * 1e3);
    }

    let start = Instant::now();
    let mut exported = Vec::new();
    repository.export(root, &mut exported).unwrap();
    let export_ms = start.elapsed().as_secs_f64() * 1e3;
    let export_bytes = exported.len() as u64;

    drop(repository);
    let start = Instant::now();
    let repository = Repository::open(project.path()).unwrap();
    let reopen_ms = start.elapsed().as_secs_f64() * 1e3;
    let start = Instant::now();
    let (selected, loaded) = repository.load().unwrap();
    assert_eq!(selected.root, root);
    assert!(loaded.object_count() > 0);
    let reopen_load_ms = start.elapsed().as_secs_f64() * 1e3;
    drop(repository);
    drop(loaded);

    // Restore into an untouched fresh store and verify exact source bytes.
    let mut restored = Repository::init(restore_dir.path()).unwrap();
    let start = Instant::now();
    let receipt = restored.import(Cursor::new(&exported), "restore").unwrap();
    let import_ms = start.elapsed().as_secs_f64() * 1e3;
    assert_eq!(receipt.after.root, root);
    let start = Instant::now();
    let (selected, restored_engine) = restored.load().unwrap();
    assert_eq!(selected.root, root);
    let entry = restored_engine
        .read(root, &[file_name(last_changed)])
        .unwrap();
    let Entry::File { blob, .. } = entry else {
        panic!("expected file entry");
    };
    let Object::Blob(got) = restored_engine.object(blob).unwrap() else {
        panic!("expected blob object");
    };
    assert_eq!(
        got,
        &content(
            last_changed as u64 * 2 + 1 + w.revisions as u64 * 1_000_003,
            w.file_bytes[last_changed],
        )
    );
    let restore_load_ms = start.elapsed().as_secs_f64() * 1e3;
    drop(restored);

    let (disk_bytes, disk_files) = disk_usage(&project.path().join(".avc"));
    let db_bytes = ["store.sqlite", "store.sqlite-wal", "store.sqlite-shm"]
        .iter()
        .map(|n| named_file_size(&project.path().join(".avc"), n))
        .sum::<u64>();
    let db = rusqlite::Connection::open_with_flags(
        project.path().join(".avc/store.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let sqlite_version: String = db
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    let journal_mode: String = db
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    let synchronous: i64 = db
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap();
    let object_rows: i64 = db
        .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
        .unwrap();

    let result = serde_json::json!({
        "label": label,
        "workload": w.name,
        "rep": rep,
        "source_rev": source_rev,
        "env": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "sqlite": sqlite_version,
        },
        "durability": {
            "journal_mode": journal_mode,
            "synchronous": synchronous,
            "note": "Repository::configure refuses any open without WAL +
                     synchronous=FULL (+ fullfsync/checkpoint_fullfsync on macOS);
                     every acknowledged commit here paid that contract.",
        },
        "workload_params": {
            "file_count": w.file_count,
            "file_bytes_total": w.file_bytes.iter().sum::<usize>(),
            "revisions": w.revisions,
            "replaced_per_rev": w.replaced_per_rev,
        },
        "content_fingerprint": fingerprint.finalize().to_hex().to_string(),
        "engine_initial_ms": initial_engine_ms,
        "engine_churn_ms": summary(&engine_ms),
        "commit_ms": commit_ms,
        "commit_summary": summary(&commit_ms),
        "read_ms": read_ms,
        "read_summary": summary(&read_ms),
        "export_ms": export_ms,
        "export_stream_bytes": export_bytes,
        "import_ms": import_ms,
        "restore_load_ms": restore_load_ms,
        "reopen_ms": reopen_ms,
        "reopen_load_ms": reopen_load_ms,
        "disk": {
            "bytes": disk_bytes,
            "files": disk_files,
            "db_bytes": db_bytes,
            "object_rows": object_rows,
        },
    });
    println!("{}", serde_json::to_string(&result).unwrap());
}
