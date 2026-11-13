//! Independent portable-origin wire construction and stock-Git/actual-CLI oracles.
use avc::{
    Entry, Id, Object, Revision,
    engine::Edit,
    git::GitOid,
    repository::{Error, Repository},
};
use rusqlite::{Connection, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Barrier},
};

fn git(dir: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().to_owned()
}
fn hash(dir: &Path, kind: &str, bytes: &[u8]) -> String {
    text(git(
        dir,
        &["hash-object", "-w", "-t", kind, "--stdin"],
        bytes,
    ))
}
fn bare(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    git(parent, &["init", "--bare", path.to_str().unwrap()], b"");
    path
}
fn objects(dir: &Path) -> BTreeSet<String> {
    text(git(
        dir,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
        b"",
    ))
    .lines()
    .map(str::to_owned)
    .collect()
}
fn cli(project: &Path, args: &[&str], path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_avc"));
    command
        .arg("--repository")
        .arg(project)
        .args(args)
        .env_remove("AVC_AUTHOR");
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.output().unwrap()
}
fn success(output: Output) -> Vec<u8> {
    assert!(output.status.success(), "{output:?}");
    output.stdout
}
fn put(repo: &mut Repository, key: &str, bytes: &[u8]) {
    let (selected, mut engine) = repo.load().unwrap();
    let path = vec![b"native".to_vec()];
    let expected = engine.read(selected.root, &path).unwrap();
    let blob = engine.blob(bytes.to_vec()).unwrap();
    let root = engine
        .edit(
            selected.root,
            &[Edit {
                path,
                expected,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    repo.commit_source(&engine, root, selected.generation, key)
        .unwrap();
}
fn author(repo: &mut Repository, key: &str) -> Id {
    repo.commit(
        "Native <native@example.test>",
        key,
        repo.status().unwrap().generation,
        key,
    )
    .unwrap()
    .after
    .revision
    .unwrap()
}
struct Fixture {
    temp: tempfile::TempDir,
    git: PathBuf,
    native: PathBuf,
    repo: Repository,
    anchor: Id,
    head: Id,
    oid: GitOid,
    raw: Vec<u8>,
    tree: String,
    frontier: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let git_dir = bare(temp.path(), "original.git");
        let mut listing = Vec::new();
        for (mode, name, bytes) in [
            ("100644", "binary", &[0, 255, 13, 10][..]),
            ("100755", "run", &b"#!/bin/sh\nexit 0\n"[..]),
            ("120000", "link", &b"binary"[..]),
        ] {
            let oid = hash(&git_dir, "blob", bytes);
            listing.extend(format!("{mode} blob {oid}\t{name}\0").as_bytes());
        }
        let blob = hash(&git_dir, "blob", b"nested\0payload");
        let subtree = text(git(
            &git_dir,
            &["mktree", "-z"],
            format!("100644 blob {blob}\tfile\0").as_bytes(),
        ));
        listing.extend(format!("040000 tree {subtree}\tdir\0").as_bytes());
        let tree = text(git(&git_dir, &["mktree", "-z"], &listing));
        // A valid foreign parent identity, deliberately never materialized.
        let frontier = "1111111111111111111111111111111111111111".to_owned();
        let raw = format!("tree {tree}\nparent {frontier}\nauthor Original <author@example.test> 1700000000 -0230\ncommitter Collector <collector@example.test> 1700000001 +0545\nx-opaque retained\n continuation\nencoding UTF-8\n\nExact λ\n\nno terminal newline").into_bytes();
        let oid = hash(&git_dir, "commit", &raw).parse().unwrap();
        let native = temp.path().join("original-native");
        fs::create_dir(&native).unwrap();
        fs::write(native.join("live"), b"untouched original").unwrap();
        let mut repo = Repository::init(&native).unwrap();
        let anchor = repo
            .import_git(&git_dir, oid, "foreign")
            .unwrap()
            .after
            .revision
            .unwrap();
        put(&mut repo, "child-source", b"authored\0child");
        let head = author(&mut repo, "child");
        put(&mut repo, "dirty-source", b"dirty selected bytes\r\n");
        Self {
            temp,
            git: git_dir,
            native,
            repo,
            anchor,
            head,
            oid,
            raw,
            tree,
            frontier,
        }
    }
}

#[derive(Clone)]
struct Claim {
    revision: Id,
    oid: GitOid,
    raw: Vec<u8>,
}
type Records = BTreeMap<Id, Vec<u8>>;
fn origin_records(repo: &Repository, root: Id, head: Option<Id>) -> Records {
    let mut records = BTreeMap::new();
    for root in std::iter::once(root).chain(head) {
        for (id, object) in repo.load_root(root).unwrap().objects() {
            records.insert(id, object.encode().unwrap());
        }
    }
    records
}
// Literal wire grammar: deliberately independent of the production writer.
fn literal(root: Id, head: Option<Id>, records: &Records, claims: Option<&[Claim]>) -> Vec<u8> {
    let mut out = if claims.is_some() {
        b"AVCHIS02".to_vec()
    } else {
        b"AVCHIS01".to_vec()
    };
    out.extend(root.to_bytes().unwrap());
    out.push(u8::from(head.is_some()));
    if let Some(head) = head {
        out.extend(head.to_bytes().unwrap());
    }
    out.extend((records.len() as u64).to_le_bytes());
    for (id, payload) in records {
        out.extend(id.to_bytes().unwrap());
        out.extend((payload.len() as u64).to_le_bytes());
        out.extend(payload);
    }
    if let Some(claims) = claims {
        out.extend((claims.len() as u64).to_le_bytes());
        for claim in claims {
            out.extend(claim.revision.to_bytes().unwrap());
            out.extend(claim.oid.to_bytes());
            out.extend((claim.raw.len() as u64).to_le_bytes());
            out.extend(&claim.raw);
        }
        out.extend(b"AVCHEND2");
    } else {
        out.extend(b"AVCHEND1");
    }
    out.extend(blake3::hash(&out).as_bytes());
    out
}

fn literal_source(root: Id, records: &Records) -> Vec<u8> {
    let mut out = b"AVCEXP01".to_vec();
    out.extend(root.to_bytes().unwrap());
    out.extend((records.len() as u64).to_le_bytes());
    for (id, payload) in records {
        out.extend(id.to_bytes().unwrap());
        out.extend((payload.len() as u64).to_le_bytes());
        out.extend(payload);
    }
    out.extend(b"AVCEND01");
    out.extend(blake3::hash(&out).as_bytes());
    out
}
fn fixture_claim(f: &Fixture) -> Claim {
    Claim {
        revision: f.anchor,
        oid: f.oid,
        raw: f.raw.clone(),
    }
}
fn fixture_wire(f: &Fixture) -> Vec<u8> {
    let selected = f.repo.status().unwrap();
    literal(
        selected.root,
        selected.revision,
        &origin_records(&f.repo, selected.root, selected.revision),
        Some(&[fixture_claim(f)]),
    )
}
fn db(project: &Path) -> Connection {
    Connection::open(project.join(".avc/store.sqlite")).unwrap()
}
// Read through SQLite connections, including during deliberate SQL-level damage.
fn durable_rows(connection: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "objects",
        "git_origins",
        "git_exports",
        "selection",
        "operations",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}
fn manifest(claims: &[Claim]) -> Vec<u8> {
    let mut out = b"AVCGOR01".to_vec();
    out.extend((claims.len() as u32).to_le_bytes());
    for claim in claims {
        out.extend(claim.revision.to_bytes().unwrap());
        out.extend(claim.oid.to_bytes());
        out.extend(blake3::hash(&claim.raw).as_bytes());
    }
    let mut hash = blake3::Hasher::new();
    hash.update(b"AVC-HISTORY-ORIGINS\0v1\0");
    hash.update(&out);
    out.extend(hash.finalize().as_bytes());
    out
}

#[test]
fn native_only_and_source_only_keep_literal_v1_bytes_and_empty_origin_scope() {
    let native = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(native.path()).unwrap();
    put(&mut repo, "source", b"native-only");
    author(&mut repo, "native-born");
    let selected = repo.status().unwrap();
    let mut wire = Vec::new();
    repo.export_history(&mut wire).unwrap();
    assert_eq!(
        wire,
        literal(
            selected.root,
            selected.revision,
            &origin_records(&repo, selected.root, selected.revision),
            None
        )
    );
    let fresh = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(fresh.path()).unwrap();
    restored.import_history(wire.as_slice(), "v1").unwrap();
    let connection = db(fresh.path());
    connection
        .execute_batch("DROP TRIGGER operations_no_update")
        .unwrap();
    assert!(
        connection
            .execute(
                "UPDATE operations SET import_origins = NULL WHERE request_key = 'v1'",
                []
            )
            .is_err()
    );
    let scope: Vec<u8> = connection
        .query_row(
            "SELECT import_origins FROM operations WHERE request_key = 'v1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scope, manifest(&[]));
    let f = Fixture::new();
    let root = f.repo.status().unwrap().root;
    let mut source = Vec::new();
    f.repo.export(root, &mut source).unwrap();
    assert_eq!(
        source,
        literal_source(root, &origin_records(&f.repo, root, None))
    );
    let plain = tempfile::tempdir().unwrap();
    let mut plain_repo = Repository::init(plain.path()).unwrap();
    plain_repo.import(source.as_slice(), "source-only").unwrap();
    let mut again = Vec::new();
    plain_repo.export(root, &mut again).unwrap();
    assert_eq!(source, again);
    assert!(plain_repo.status().unwrap().revision.is_none());
    let connection = db(plain.path());
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM git_origins", [], |r| r.get(0))
            .unwrap(),
        0
    );
    connection
        .execute_batch("DROP TRIGGER operations_no_update")
        .unwrap();
    assert!(
        connection
            .execute(
                "UPDATE operations SET import_origins = ?1 WHERE request_key = 'source-only'",
                [manifest(&[])]
            )
            .is_err()
    );
    assert!(
        connection
            .query_row::<Option<Vec<u8>>, _, _>(
                "SELECT import_origins FROM operations WHERE request_key = 'source-only'",
                [],
                |r| r.get(0)
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn complete_origin_scope_is_retry_identity_not_current_origin_health() {
    let f = Fixture::new();
    let wire = fixture_wire(&f);
    let selected = f.repo.status().unwrap();
    let records = origin_records(&f.repo, selected.root, selected.revision);
    let v1 = literal(selected.root, selected.revision, &records, None);
    for missing in [false, true] {
        let fresh = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(fresh.path()).unwrap();
        let receipt = repo.import_history(wire.as_slice(), "restore").unwrap();
        let connection = db(fresh.path());
        assert_eq!(
            connection
                .pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            11
        );
        let scope: Vec<u8> = connection
            .query_row(
                "SELECT import_origins FROM operations WHERE request_key = 'restore'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(scope, manifest(&[fixture_claim(&f)]));
        assert_eq!(scope.len(), 8 + 4 + 86 + 32);
        let retained: Vec<u8> = connection
            .query_row("SELECT raw_commit FROM git_origins", [], |r| r.get(0))
            .unwrap();
        assert_eq!(retained, f.raw);
        put(&mut repo, "later-source", b"later edits");
        author(&mut repo, "later-author");
        let current = repo.status().unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER git_origins_no_update; DROP TRIGGER git_origins_no_delete;",
            )
            .unwrap();
        if missing {
            connection.execute("DELETE FROM git_origins", []).unwrap();
        } else {
            connection
                .execute("UPDATE git_origins SET raw_commit = X'00'", [])
                .unwrap();
        }
        let before = durable_rows(&connection);
        assert_eq!(
            repo.import_history(wire.as_slice(), "restore").unwrap(),
            receipt
        );
        assert_eq!(repo.receipt("restore").unwrap(), Some(receipt));
        assert_eq!(repo.status().unwrap(), current);
        assert_eq!(durable_rows(&connection), before); // ACK recovery does not repair custody.
        assert!(matches!(
            repo.import_history(v1.as_slice(), "restore"),
            Err(Error::RequestKeyReuse(_))
        ));
        assert_eq!(durable_rows(&connection), before);
        assert!(repo.git_origin(f.anchor).is_err());
        assert!(repo.git_origins(f.head).is_err());
    }
    // Both streams are independently valid, with exactly the same source/head key.
    let fresh = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(fresh.path()).unwrap();
    let receipt = repo.import_history(v1.as_slice(), "restore").unwrap();
    assert!(matches!(
        repo.import_history(wire.as_slice(), "restore"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(
        repo.import_history(v1.as_slice(), "restore").unwrap(),
        receipt
    );
    assert!(repo.git_origins(f.head).unwrap().is_empty());
}

#[test]
fn damaged_origin_custody_never_becomes_native_root_or_writes_outputs() {
    let f = Fixture::new();
    let root = f.repo.revision(f.anchor).unwrap().root;
    let input = literal(
        root,
        Some(f.anchor),
        &origin_records(&f.repo, root, Some(f.anchor)),
        Some(&[fixture_claim(&f)]),
    );
    for damage in ["missing raw", "corrupt raw", "manifest bitrot"] {
        let fresh = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(fresh.path()).unwrap();
        repo.import_history(input.as_slice(), "restore").unwrap();
        let connection = db(fresh.path());
        connection.execute_batch("DROP TRIGGER git_origins_no_update; DROP TRIGGER git_origins_no_delete; DROP TRIGGER operations_no_update;").unwrap();
        match damage {
            "missing raw" => {
                connection.execute("DELETE FROM git_origins", []).unwrap();
            }
            "corrupt raw" => {
                connection
                    .execute("UPDATE git_origins SET raw_commit = X'00'", [])
                    .unwrap();
            }
            _ => {
                let mut scope = manifest(&[fixture_claim(&f)]);
                scope[20] ^= 1;
                connection
                    .execute(
                        "UPDATE operations SET import_origins = ?1 WHERE request_key = 'restore'",
                        [scope],
                    )
                    .unwrap();
                assert!(
                    repo.import_history(input.as_slice(), "restore").is_err(),
                    "bitrot must not recover a root/head-only ACK"
                );
            }
        }
        let before = durable_rows(&connection);
        let selected = repo.status().unwrap();
        assert!(repo.git_origin(f.anchor).is_err(), "{damage}");
        assert!(repo.git_origins(f.anchor).is_err(), "{damage}");
        let mut output = Vec::new();
        assert!(repo.export_history(&mut output).is_err(), "{damage}");
        assert!(output.is_empty());
        let history = fresh.path().join("must-not-create.avc");
        assert!(repo.export_history_file(&history).is_err(), "{damage}");
        assert!(!history.exists());
        let target = bare(fresh.path(), "target.git");
        assert!(repo.export_git(&target, f.anchor).is_err(), "{damage}");
        assert!(objects(&target).is_empty());
        assert!(
            repo.amend(
                "Native <native@example.test>",
                "no",
                selected.generation,
                "amend"
            )
            .is_err(),
            "{damage}"
        );
        assert!(
            repo.rebase(f.anchor, selected.generation, "rebase")
                .is_err(),
            "{damage}"
        );
        assert_eq!(repo.status().unwrap(), selected);
        assert_eq!(durable_rows(&connection), before);
    }
}

#[test]
fn denied_origin_history_receipt_rolls_back_every_admission_and_can_retry() {
    let f = Fixture::new();
    let input = fixture_wire(&f);
    let fresh = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(fresh.path()).unwrap();
    let connection = db(fresh.path());
    let before = durable_rows(&connection);
    connection.execute_batch("CREATE TRIGGER deny_history BEFORE INSERT ON operations WHEN NEW.kind = 'import-history' BEGIN SELECT RAISE(ABORT, 'denied history'); END;").unwrap();
    let error = repo
        .import_history(input.as_slice(), "restore")
        .unwrap_err()
        .to_string();
    assert!(error.contains("denied history"), "{error}");
    assert_eq!(durable_rows(&connection), before);
    assert_eq!(repo.status().unwrap().generation, 0);
    assert!(repo.receipt("restore").unwrap().is_none());
    connection
        .execute_batch("DROP TRIGGER deny_history")
        .unwrap();
    repo.import_history(input.as_slice(), "restore").unwrap();
    assert_eq!(
        repo.git_origin(f.anchor).unwrap().unwrap().raw_commit,
        f.raw
    );
}

#[test]
fn concurrent_origin_history_import_has_one_receipt_or_one_not_empty() {
    let f = Fixture::new();
    let input = fixture_wire(&f);
    for same_key in [false, true] {
        let fresh = tempfile::tempdir().unwrap();
        drop(Repository::init(fresh.path()).unwrap());
        // All connections finish open/setup before either worker can publish.
        let handles = (0..2)
            .map(|_| Repository::open(fresh.path()).unwrap())
            .collect::<Vec<_>>();
        let barrier = Arc::new(Barrier::new(2));
        let workers = handles
            .into_iter()
            .enumerate()
            .map(|(i, mut repo)| {
                let barrier = barrier.clone();
                let input = input.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    repo.import_history(
                        input.as_slice(),
                        if same_key || i == 0 { "one" } else { "two" },
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .collect::<Vec<_>>();
        if same_key {
            assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        } else {
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert!(results.iter().any(|r| matches!(r, Err(Error::NotEmpty))));
        }
        let repo = Repository::open(fresh.path()).unwrap();
        assert_eq!(repo.status().unwrap().generation, 1);
        assert_eq!(repo.git_origins(f.head).unwrap().len(), 1);
        assert_eq!(
            db(fresh.path())
                .query_row::<i64, _, _>("SELECT count(*) FROM operations", [], |r| r.get(0))
                .unwrap(),
            1
        );
    }
}

fn bind_anchor(template: &Revision, oid: GitOid) -> Revision {
    let mut revision = template.clone();
    let mut domain = b"AVC-GIT-ANCHOR\0v1\0".to_vec();
    domain.extend([1, 20]);
    domain.extend(&oid.to_bytes()[1..]);
    revision
        .change
        .copy_from_slice(&blake3::hash(&domain).as_bytes()[..16]);
    revision
}
fn add_revision(records: &mut Records, revision: Revision) -> Id {
    let object = Object::Revision(revision);
    let id = object.id().unwrap();
    records.insert(id, object.encode().unwrap());
    id
}
fn resign(bytes: &mut [u8]) {
    let end = bytes.len() - 32;
    let checksum = blake3::hash(&bytes[..end]);
    bytes[end..].copy_from_slice(checksum.as_bytes());
}

#[test]
fn literal_v2_rejects_malformed_origins_and_source_binding_before_any_admission() {
    let f = Fixture::new();
    let selected = f.repo.status().unwrap();
    let records = origin_records(&f.repo, selected.root, selected.revision);
    let claim = fixture_claim(&f);
    let valid = literal(
        selected.root,
        selected.revision,
        &records,
        Some(std::slice::from_ref(&claim)),
    );
    let v1 = literal(selected.root, selected.revision, &records, None);
    let origin_offset = v1.len() - 40;
    assert_eq!(&valid[8..origin_offset], &v1[8..origin_offset]);
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    cases.push((
        "zero origins",
        literal(selected.root, selected.revision, &records, Some(&[])),
    ));
    cases.push((
        "duplicate origins",
        literal(
            selected.root,
            selected.revision,
            &records,
            Some(&[claim.clone(), claim.clone()]),
        ),
    ));
    let mut oversized_count = valid.clone();
    oversized_count[origin_offset..origin_offset + 8].copy_from_slice(&100001u64.to_le_bytes());
    resign(&mut oversized_count);
    cases.push(("overbound count", oversized_count));
    let mut enormous_count = valid.clone();
    enormous_count[origin_offset..origin_offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    resign(&mut enormous_count);
    cases.push(("overflow count", enormous_count));
    let mut oversized_raw = valid.clone();
    let raw_len_offset = origin_offset + 8 + 33 + 21;
    oversized_raw[raw_len_offset..raw_len_offset + 8].copy_from_slice(&16777217u64.to_le_bytes());
    resign(&mut oversized_raw);
    cases.push(("overbound raw length", oversized_raw));
    let mut unknown_algorithm = valid.clone();
    unknown_algorithm[origin_offset + 8 + 33] = 2;
    resign(&mut unknown_algorithm);
    cases.push(("unknown OID algorithm", unknown_algorithm));
    let mut invalid_kind = valid.clone();
    invalid_kind[origin_offset + 8] = selected.root.to_bytes().unwrap()[0];
    resign(&mut invalid_kind);
    cases.push(("nonrevision origin ID", invalid_kind));
    let mut wrong_raw = claim.clone();
    wrong_raw.raw.push(b'!');
    cases.push((
        "raw OID mismatch",
        literal(
            selected.root,
            selected.revision,
            &records,
            Some(&[wrong_raw]),
        ),
    ));
    let outside = Claim {
        revision: f.head,
        ..claim.clone()
    };
    let anchor_root = f.repo.revision(f.anchor).unwrap().root;
    let anchor_records = origin_records(&f.repo, anchor_root, Some(f.anchor));
    cases.push((
        "origin outside head ancestry",
        literal(
            anchor_root,
            Some(f.anchor),
            &anchor_records,
            Some(&[outside]),
        ),
    ));
    cases.push((
        "origin with nonzero native parents",
        literal(
            selected.root,
            selected.revision,
            &records,
            Some(&[Claim {
                revision: f.head,
                ..claim.clone()
            }]),
        ),
    ));
    let template = f.repo.revision(f.anchor).unwrap();
    let mut unbound = template.clone();
    unbound.author = "Different <different@example.test>".into();
    let mut unbound_records = anchor_records.clone();
    unbound_records.remove(&f.anchor);
    let unbound_id = add_revision(&mut unbound_records, unbound);
    cases.push((
        "full anchor metadata mismatch",
        literal(
            anchor_root,
            Some(unbound_id),
            &unbound_records,
            Some(&[Claim {
                revision: unbound_id,
                ..claim.clone()
            }]),
        ),
    ));
    for (name, raw) in [
        (
            "invalid Git metadata with valid raw OID",
            String::from_utf8(f.raw.clone())
                .unwrap()
                .replace("1700000001 +0545", "not-a-time +0545")
                .into_bytes(),
        ),
        (
            "source tree mismatch with valid OID and canonical anchor",
            {
                let empty_tree = text(git(&f.git, &["mktree"], b""));
                String::from_utf8(f.raw.clone())
                    .unwrap()
                    .replace(&f.tree, &empty_tree)
                    .into_bytes()
            },
        ),
    ] {
        // --literally permits the intentionally malformed metadata while stock
        // Git still supplies an independent SHA-1 object-framing oracle.
        let oid = text(git(
            &f.git,
            &[
                "hash-object",
                "--literally",
                "-w",
                "-t",
                "commit",
                "--stdin",
            ],
            &raw,
        ))
        .parse()
        .unwrap();
        let revision = bind_anchor(&template, oid);
        let mut altered = anchor_records.clone();
        altered.remove(&f.anchor);
        let id = add_revision(&mut altered, revision);
        cases.push((
            name,
            literal(
                anchor_root,
                Some(id),
                &altered,
                Some(&[Claim {
                    revision: id,
                    oid,
                    raw,
                }]),
            ),
        ));
    }
    let mut checksum = valid.clone();
    *checksum.last_mut().unwrap() ^= 1;
    cases.push(("checksum", checksum));
    let mut footer = valid.clone();
    let footer_offset = footer.len() - 40;
    footer[footer_offset + 7] = b'1';
    resign(&mut footer);
    cases.push(("wrong footer", footer));
    cases.push(("truncated checksum", valid[..valid.len() - 1].to_vec()));
    cases.push(("truncated raw", valid[..raw_len_offset + 9].to_vec()));
    let mut trailing = valid.clone();
    trailing.push(0);
    cases.push(("trailing bytes", trailing));
    // Two independently canonical anchors under one native merge isolate order.
    let mut raw2 = f.raw.clone();
    raw2.extend(b" second anchor");
    let oid2 = hash(&f.git, "commit", &raw2).parse().unwrap();
    let mut multi = anchor_records.clone();
    let mut anchor2 = bind_anchor(&template, oid2);
    anchor2.message.push_str(" second anchor");
    let id2 = add_revision(&mut multi, anchor2);
    let mut merge = template.clone();
    merge.parents = vec![f.anchor, id2];
    merge.change = [93; 16];
    let merge_id = add_revision(&mut multi, merge);
    let mut claims = vec![
        claim,
        Claim {
            revision: id2,
            oid: oid2,
            raw: raw2,
        },
    ];
    claims.sort_by_key(|c| c.revision);
    let ordered = literal(anchor_root, Some(merge_id), &multi, Some(&claims));
    // Positive controls prevent an unsupported v2 reader from passing negatives.
    // Construct every stock-Git fixture first so baseline runs also check them.
    let control = tempfile::tempdir().unwrap();
    Repository::init(control.path())
        .unwrap()
        .import_history(valid.as_slice(), "valid")
        .unwrap();
    let ordered_target = tempfile::tempdir().unwrap();
    let mut ordered_repo = Repository::init(ordered_target.path()).unwrap();
    ordered_repo
        .import_history(ordered.as_slice(), "ordered")
        .unwrap();
    let mut ordered_export = Vec::new();
    ordered_repo.export_history(&mut ordered_export).unwrap();
    assert_eq!(ordered_export, ordered);
    claims.reverse();
    cases.push((
        "unsorted origins",
        literal(anchor_root, Some(merge_id), &multi, Some(&claims)),
    ));
    let fresh = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(fresh.path()).unwrap();
    let connection = db(fresh.path());
    let before = durable_rows(&connection);
    for (name, bytes) in cases {
        assert!(
            repo.import_history(bytes.as_slice(), "bad").is_err(),
            "accepted {name}"
        );
        assert_eq!(durable_rows(&connection), before, "admitted {name}");
        assert_eq!(repo.status().unwrap().generation, 0, "{name}");
        assert!(repo.receipt("bad").unwrap().is_none(), "{name}");
    }
    repo.import_history(valid.as_slice(), "bad").unwrap();
}

fn loose_path(git_dir: &Path, oid: &str) -> PathBuf {
    git_dir.join("objects").join(&oid[..2]).join(&oid[2..])
}
fn loose_files(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(loose_files(&path));
        } else {
            result.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    result
}

#[test]
fn foreign_target_preflight_refuses_corruption_before_child_writes_but_fills_missing_closure() {
    let mut f = Fixture::new();
    let wire = fixture_wire(&f);
    let original_objects = objects(&f.git);
    let original = f.oid.to_string().replace("sha1:", "");
    let blob = text(git(
        &f.git,
        &["rev-parse", &format!("{original}:binary")],
        b"",
    ));
    let expected = f.repo.export_git(&f.git, f.head).unwrap();
    for oid in [&original, &f.tree, &blob] {
        for corrupt in [false, true] {
            let fresh = tempfile::tempdir().unwrap();
            let mut repo = Repository::init(fresh.path()).unwrap();
            repo.import_history(wire.as_slice(), "restore").unwrap();
            let target = bare(fresh.path(), "target.git");
            // Seed with stock Git, not the export implementation under test.
            for object in &original_objects {
                let kind = text(git(&f.git, &["cat-file", "-t", object], b""));
                let bytes = git(&f.git, &["cat-file", &kind, object], b"");
                assert_eq!(hash(&target, &kind, &bytes), *object);
            }
            let path = loose_path(&target, oid);
            // Stock Git installs loose objects read-only. Replace this owned
            // fixture inode instead of assuming its mode permits overwriting.
            fs::remove_file(&path).unwrap();
            if corrupt {
                fs::write(path, b"not a valid loose Git object").unwrap();
            }
            let selected = repo.status().unwrap();
            let before = durable_rows(&db(fresh.path()));
            let files = loose_files(&target.join("objects"));
            if corrupt {
                assert!(repo.export_git(&target, f.head).is_err(), "corrupt {oid}");
                assert_eq!(
                    loose_files(&target.join("objects")),
                    files,
                    "wrote before rejecting {oid}"
                );
                assert_eq!(durable_rows(&db(fresh.path())), before);
            } else {
                assert_eq!(
                    repo.export_git(&target, f.head).unwrap(),
                    expected,
                    "missing {oid}"
                );
                assert_eq!(git(&target, &["cat-file", "commit", &original], b""), f.raw);
                assert_eq!(objects(&target), objects(&f.git));
            }
            assert_eq!(repo.status().unwrap(), selected);
            assert_eq!(text(git(&target, &["for-each-ref"], b"")), "");
        }
    }
}

#[test]
fn exact_git_empty_directories_rehydrate_but_native_unchanged_child_still_obeys_policy_one() {
    let f = Fixture::new();
    let empty = text(git(&f.git, &["mktree"], b""));
    let nested = text(git(
        &f.git,
        &["mktree", "-z"],
        format!("040000 tree {empty}\tinside\0").as_bytes(),
    ));
    let mut listing = git(&f.git, &["ls-tree", "-z", &f.tree], b"");
    listing.extend(format!("040000 tree {nested}\tempty-nested\0").as_bytes());
    let tree = text(git(&f.git, &["mktree", "-z"], &listing));
    let raw = String::from_utf8(f.raw.clone())
        .unwrap()
        .replace(&f.tree, &tree)
        .into_bytes();
    let oid = hash(&f.git, "commit", &raw).parse().unwrap();
    let original = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(original.path()).unwrap();
    let anchor = repo
        .import_git(&f.git, oid, "foreign-empty")
        .unwrap()
        .after
        .revision
        .unwrap();
    let mut wire = Vec::new();
    repo.export_history(&mut wire).unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(fresh.path()).unwrap();
    restored.import_history(wire.as_slice(), "restore").unwrap();
    let target = bare(fresh.path(), "target.git");
    assert_eq!(restored.export_git(&target, anchor).unwrap(), oid);
    assert_eq!(
        git(
            &target,
            &["cat-file", "commit", &oid.to_string().replace("sha1:", "")],
            b""
        ),
        raw
    );
    assert_eq!(
        git(&target, &["ls-tree", "-rz", &tree], b""),
        git(&f.git, &["ls-tree", "-rz", &tree], b"")
    );
    // Origin reconstruction is not a cache exemption for native policy v1.
    let child = author(&mut restored, "unchanged-native-child");
    assert_eq!(
        restored.revision(child).unwrap().root,
        restored.revision(anchor).unwrap().root
    );
    let before = loose_files(&target.join("objects"));
    for cached in [false, true] {
        if cached {
            db(fresh.path()).execute("INSERT INTO git_exports (revision, algorithm, policy, commit_oid) VALUES (?1, 1, 1, ?2)", params![child.to_bytes().unwrap().as_slice(), oid.to_bytes().as_slice()]).unwrap();
        }
        let error = restored.export_git(&target, child).unwrap_err().to_string();
        assert!(error.contains("empty director"), "{error}");
        assert_eq!(loose_files(&target.join("objects")), before);
    }
}

#[test]
fn portable_git_origin_history_survives_both_original_stores_and_actual_offline_cli() {
    let mut f = Fixture::new();
    let selected = f.repo.status().unwrap();
    let log = f.repo.log(None, 20).unwrap();
    let origin = f.repo.git_origin(f.anchor).unwrap().unwrap();
    let mut wire = Vec::new();
    f.repo
        .export_history(&mut wire)
        .expect("known Git-origin history must export as portable AVCHIS02");
    assert_eq!(&wire[..8], b"AVCHIS02");
    assert_eq!(wire, fixture_wire(&f));
    let oracle_candidate = f.repo.export_git(&f.git, f.head).unwrap();
    let oracle_objects = objects(&f.git);
    let mut source = Vec::new();
    f.repo.export(selected.root, &mut source).unwrap();
    assert_eq!(&source[..8], b"AVCEXP01");
    let input = f.temp.path().join("portable.avc");
    fs::write(&input, &wire).unwrap();
    if std::env::var_os("AVC_REFERENCE_CHECK").is_some() {
        let output = Command::new("uv")
            .args(["run", "--offline"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("reference/bundle.py"))
            .arg("--history")
            .arg(&input)
            .arg("--json")
            .output()
            .unwrap();
        let stdout = success(output);
        let corpus: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(
            corpus["roots"],
            serde_json::json!([selected.root.to_string(), f.head.to_string()])
        );
        let raw_hex: String = f.raw.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            corpus["origins"],
            serde_json::json!([{
                "revision": f.anchor.to_string(), "commit": f.oid.to_string(), "raw_commit": raw_hex,
            }])
        );
    }
    let cli_wire = f.temp.path().join("cli-history.avc");
    success(cli(
        &f.native,
        &[
            "export",
            "--history",
            "--output",
            cli_wire.to_str().unwrap(),
        ],
        None,
    ));
    assert_eq!(fs::read(cli_wire).unwrap(), wire);
    let fresh = f.temp.path().join("fresh");
    fs::create_dir(&fresh).unwrap();
    fs::write(fresh.join("live"), b"untouched restored").unwrap();
    let empty_path = f.temp.path().join("empty-path");
    fs::create_dir(&empty_path).unwrap();
    drop(Repository::init(&fresh).unwrap());
    let head = f.head;
    let anchor = f.anchor;
    let oid = f.oid;
    let raw = f.raw;
    let tree = f.tree;
    let frontier = f.frontier;
    drop(f.repo);
    fs::remove_dir_all(&f.git).unwrap();
    fs::remove_dir_all(&f.native).unwrap();
    success(cli(
        &fresh,
        &[
            "--experimental",
            "import",
            "--history",
            "--input",
            input.to_str().unwrap(),
            "--request-key",
            "restore",
        ],
        Some(&empty_path),
    ));
    let mut restored = Repository::open(&fresh).unwrap();
    let now = restored.status().unwrap();
    assert_eq!(
        (now.root, now.revision, now.generation),
        (selected.root, selected.revision, 1)
    );
    assert_eq!(restored.log(None, 20).unwrap(), log);
    assert_eq!(restored.git_origin(anchor).unwrap(), Some(origin.clone()));
    assert_eq!(restored.git_origins(head).unwrap(), vec![origin]);
    let mut again = Vec::new();
    restored.export_history(&mut again).unwrap();
    assert_eq!(again, wire);
    let mut source_again = Vec::new();
    restored.export(now.root, &mut source_again).unwrap();
    assert_eq!(source_again, source);
    let target = bare(f.temp.path(), "empty-target.git");
    assert!(objects(&target).is_empty());
    assert!(
        !cli(
            &fresh,
            &[
                "--experimental",
                "git",
                "export",
                "--git-dir",
                target.to_str().unwrap()
            ],
            None
        )
        .status
        .success()
    );
    assert!(objects(&target).is_empty());
    let output = success(cli(
        &fresh,
        &[
            "--experimental",
            "git",
            "export",
            "--git-dir",
            target.to_str().unwrap(),
            "--revision",
            &head.to_string(),
        ],
        None,
    ));
    let candidate = restored.export_git(&target, head).unwrap();
    assert_eq!(candidate, oracle_candidate);
    assert_eq!(objects(&target), oracle_objects);
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains(&format!("commit={candidate}"))
    );
    let candidate_hex = candidate.to_string().replace("sha1:", "");
    let original_hex = oid.to_string().replace("sha1:", "");
    assert_eq!(
        text(git(
            &target,
            &["show", "-s", "--format=%P", &candidate_hex],
            b""
        )),
        original_hex
    );
    assert_eq!(
        git(&target, &["cat-file", "commit", &original_hex], b""),
        raw
    );
    assert_eq!(
        text(git(
            &target,
            &["rev-parse", &format!("{original_hex}^{{tree}}")],
            b""
        )),
        tree
    );
    assert_eq!(
        git(&target, &["show", &format!("{original_hex}:binary")], b""),
        [0, 255, 13, 10]
    );
    assert_eq!(
        git(&target, &["show", &format!("{original_hex}:dir/file")], b""),
        b"nested\0payload"
    );
    assert_eq!(
        git(&target, &["show", &format!("{candidate_hex}:native")], b""),
        b"authored\0child"
    );
    assert!(!objects(&target).contains(&frontier));
    assert_eq!(text(git(&target, &["for-each-ref"], b"")), "");
    assert_eq!(fs::read(fresh.join("live")).unwrap(), b"untouched restored");
    assert!(!fresh.join("native").exists());
    assert_eq!(restored.status().unwrap(), now);
    // Select the retained boundary itself, then exercise mutation guards there.
    let boundary_records = origin_records(&restored, now.root, Some(anchor));
    drop(restored);
    let boundary = f.temp.path().join("boundary");
    fs::create_dir(&boundary).unwrap();
    let mut boundary_repo = Repository::init(&boundary).unwrap();
    let boundary_wire = literal(
        now.root,
        Some(anchor),
        &boundary_records,
        Some(&[Claim {
            revision: anchor,
            oid,
            raw: raw.clone(),
        }]),
    );
    boundary_repo
        .import_history(boundary_wire.as_slice(), "boundary")
        .unwrap();
    assert!(
        boundary_repo
            .amend("Native <native@example.test>", "no", 1, "amend")
            .is_err()
    );
    assert!(boundary_repo.rebase(anchor, 1, "rebase").is_err());
}
