use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let path = std::env::temp_dir().join(format!(
            "avc-retirement-{}-{}-{}",
            std::process::id(),
            nonce.as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        for directory in [".git", ".avc", "files"] {
            fs::create_dir(path.join(directory)).unwrap();
        }
        for (name, data) in [
            (".git/HEAD", b"ref: refs/heads/main\n".as_slice()),
            (".avc/oplog", b"private legacy recovery data\n"),
            ("files/edited.txt", b"uncommitted edits\n"),
            ("new secret.txt", b"not selected for authoring\n"),
        ] {
            fs::write(path.join(name), data).unwrap();
        }
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_avc"))
            .args(args)
            .current_dir(&self.0)
            .env("PATH", "")
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn snapshot(root: &Path, directory: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let relative = path.strip_prefix(root).unwrap().to_owned();
        if path.is_dir() {
            entries.push((relative, None));
            entries.extend(snapshot(root, &path));
        } else {
            entries.push((relative, Some(fs::read(path).unwrap())));
        }
    }
    entries.sort();
    entries
}

#[test]
fn operational_invocations_refuse_without_touching_repository_or_recovery_data() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.0, &fixture.0);
    for args in [
        vec![],
        vec!["init"],
        vec!["save", "-m", "do not author"],
        vec!["save", "--amend", "-m", "do not rewrite"],
        vec!["undo"],
        vec!["redo"],
        vec!["run", "--", "do-not-execute"],
        vec!["watch", "--interval", "0"],
        vec!["status"],
        vec!["log"],
        vec!["stack"],
        vec!["commit", "-m", "not implemented"],
        vec!["--json", "status"],
        vec!["--help", "save"],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
        assert_eq!(snapshot(&fixture.0, &fixture.0), before, "{args:?}");
    }
}

#[test]
fn help_and_version_are_available_without_git() {
    let fixture = Fixture::new();
    for args in [["--help"], ["-h"], ["--version"], ["-V"]] {
        let output = fixture.run(&args);
        assert!(output.status.success(), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
        assert!(!output.stdout.is_empty(), "{args:?}");
    }
}
