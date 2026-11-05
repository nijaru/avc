//! Thin experimental source CLI; no workspace scanner or implicit authoring.
use avc::engine::{Edit, Engine, Limits, MergeBudget, Relocate};
use avc::repository::{Receipt, Repository, Selection};
use avc::{Entry, Id, Object};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use usage_rs::{Args, Cli, Subcommands};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Experimental native source control. Not qualified for important-source custody.
#[derive(Cli)]
#[usage(bin = "avc", version)]
struct CommandLine {
    /// Project directory containing .avc (no upward discovery).
    #[usage(long, global, default = ".")]
    repository: PathBuf,
    /// Explicitly opt into experimental repository mutations.
    #[usage(long, global)]
    experimental: bool,
    #[usage(subcommand)]
    command: Command,
}
#[derive(Subcommands)]
enum Command {
    Init(InitArgs),
    Status(StatusArgs),
    Show(Show),
    Diff(Diff),
    Verify(Verify),
    Export(Export),
    Import(Import),
    Receipt(ReceiptArgs),
    Source(Source),
}
/// Initialize without reading or changing existing project files.
#[derive(Args)]
struct InitArgs;
/// Inspect local selection metadata; does not scan or checkpoint files.
#[derive(Args)]
struct StatusArgs;
/// Inspect exact source without a checkout. Omit path to list the root.
#[derive(Args)]
struct Show {
    #[usage(long)]
    root: Option<Id>,
    path: Option<String>,
}
/// Compare two exact source roots; no filesystem scan.
#[derive(Args)]
struct Diff {
    left: Id,
    right: Id,
}
/// Verify complete selected source closure (not power-loss qualification).
#[derive(Args)]
struct Verify {
    #[usage(long)]
    root: Option<Id>,
}
/// Export selected source only; excludes private operations and discarded roots.
#[derive(Args)]
struct Export {
    #[usage(long)]
    root: Option<Id>,
    #[usage(long)]
    output: PathBuf,
}
/// Restore an object-only export into an untouched initialized store.
#[derive(Args)]
struct Import {
    #[usage(long)]
    input: PathBuf,
    #[usage(long)]
    request_key: String,
}
/// Inspect an immutable request outcome after a lost response.
#[derive(Args)]
struct ReceiptArgs {
    request_key: String,
}
/// Checkout-free conditional source operations. These do not author revisions.
#[derive(Args)]
struct Source {
    #[usage(subcommand)]
    command: SourceCommand,
}
#[derive(Subcommands)]
enum SourceCommand {
    Put(Put),
    /// Create an exact empty source directory without changing live files.
    Mkdir(Remove),
    Remove(Remove),
    Move(Move),
    Copy(Move),
    Merge(Merge),
}
/// Replace one entry using explicit file bytes and an exact selected generation.
#[derive(Args)]
struct Put {
    path: String,
    #[usage(long)]
    file: PathBuf,
    #[usage(long)]
    executable: bool,
    /// Interpret the supplied bytes as a source symlink target; never follow it.
    #[usage(long, conflicts("--executable"))]
    symlink: bool,
    #[usage(long)]
    base: Option<Id>,
    #[usage(long)]
    expected_generation: u64,
    #[usage(long)]
    request_key: String,
}
/// Delete exactly one entry/subtree, not a live filesystem path.
#[derive(Args)]
struct Remove {
    path: String,
    #[usage(long)]
    base: Option<Id>,
    #[usage(long)]
    expected_generation: u64,
    #[usage(long)]
    request_key: String,
}
/// Relocate an immutable subtree; live filesystem files remain untouched.
#[derive(Args)]
struct Move {
    source: String,
    destination: String,
    #[usage(long)]
    overwrite: bool,
    #[usage(long)]
    base: Option<Id>,
    #[usage(long)]
    expected_generation: u64,
    #[usage(long)]
    request_key: String,
}
/// Compose three source roots with an explicit comparison base (not ancestry).
#[derive(Args)]
struct Merge {
    base: Id,
    left: Id,
    right: Id,
    #[usage(long)]
    expected_generation: u64,
    #[usage(long)]
    request_key: String,
}
fn source_path(path: Option<&str>) -> Result<Vec<Vec<u8>>> {
    let components = match path {
        None | Some("") => Vec::new(),
        Some(path) => path.split('/').map(|s| s.as_bytes().to_vec()).collect(),
    };
    avc::validate_path(&components)?;
    Ok(components)
}
fn selected(repo: &Repository, root: Option<Id>) -> Result<Id> {
    Ok(root.unwrap_or(repo.status()?.root))
}
fn entry_text(entry: &Entry) -> String {
    match entry {
        Entry::Absent => "absent".into(),
        Entry::File { blob, executable } => {
            format!("file{} {blob}", if *executable { "+x" } else { "" })
        }
        Entry::Symlink(id) => format!("symlink {id}"),
        Entry::Directory(id) => format!("directory {id}"),
        Entry::Conflict(id) => format!("conflict {id}"),
    }
}
fn display_path(path: &[Vec<u8>]) -> String {
    path.iter()
        .map(|s| s.escape_ascii().to_string())
        .collect::<Vec<_>>()
        .join("/")
}
fn print_receipt(receipt: &Receipt) {
    println!(
        "root={} generation={} request={}",
        receipt.after.root, receipt.after.generation, receipt.request_key
    );
}
fn mutation_base(
    repo: &Repository,
    base: Option<Id>,
    expected: u64,
    key: &str,
) -> Result<(Id, Engine)> {
    let prior = repo.receipt(key)?;
    let previous = prior.as_ref().map(|r| r.before).unwrap_or(repo.status()?);
    if previous.generation != expected {
        return Err(format!(
            "stale selection: expected {expected}, actual {}",
            previous.generation
        )
        .into());
    }
    let root = base.unwrap_or(previous.root);
    Ok((root, repo.load_root(root)?))
}
fn file_bytes(path: &Path) -> Result<Vec<u8>> {
    // Input is an explicitly named byte supplier, not a simultaneous workspace
    // snapshot. Read bound applies even when metadata changes or is misleading.
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err("input must be an ordinary file".into());
    }
    let mut bytes = Vec::new();
    file.take((avc::repository::MAX_OBJECT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > avc::repository::MAX_OBJECT_BYTES {
        return Err("input exceeds experimental object limit".into());
    }
    Ok(bytes)
}
fn source(repo: &mut Repository, command: SourceCommand) -> Result<()> {
    let receipt = match command {
        SourceCommand::Put(args) => {
            let (base, mut engine) =
                mutation_base(repo, args.base, args.expected_generation, &args.request_key)?;
            let path = source_path(Some(&args.path))?;
            let expected = engine.read(base, &path)?;
            let blob = engine.blob(file_bytes(&args.file)?)?;
            let value = if args.symlink {
                Entry::Symlink(blob)
            } else {
                Entry::File {
                    blob,
                    executable: args.executable,
                }
            };
            let root = engine.edit(
                base,
                &[Edit {
                    path,
                    expected,
                    value,
                }],
            )?;
            repo.commit_source(&engine, root, args.expected_generation, &args.request_key)?
        }
        SourceCommand::Mkdir(args) => {
            let (base, mut engine) =
                mutation_base(repo, args.base, args.expected_generation, &args.request_key)?;
            let path = source_path(Some(&args.path))?;
            if engine.read(base, &path)? != Entry::Absent {
                return Err("mkdir requires an absent entry".into());
            }
            let empty = engine.empty()?;
            let root = engine.edit(
                base,
                &[Edit {
                    path,
                    expected: Entry::Absent,
                    value: Entry::Directory(empty),
                }],
            )?;
            repo.commit_source(&engine, root, args.expected_generation, &args.request_key)?
        }
        SourceCommand::Remove(args) => {
            let (base, mut engine) =
                mutation_base(repo, args.base, args.expected_generation, &args.request_key)?;
            let path = source_path(Some(&args.path))?;
            let expected = engine.read(base, &path)?;
            if expected == Entry::Absent {
                return Err("path is already absent".into());
            }
            let root = engine.edit(
                base,
                &[Edit {
                    path,
                    expected,
                    value: Entry::Absent,
                }],
            )?;
            repo.commit_source(&engine, root, args.expected_generation, &args.request_key)?
        }
        SourceCommand::Move(args) => relocate(repo, args, false)?,
        SourceCommand::Copy(args) => relocate(repo, args, true)?,
        SourceCommand::Merge(args) => {
            let mut engine = repo.load_root(args.base)?;
            for root in [args.left, args.right] {
                let other = repo.load_root(root)?;
                engine.import(
                    other
                        .objects()
                        .map(|(id, o)| Ok((id, o.encode()?)))
                        .collect::<avc::Result<Vec<_>>>()?,
                    root,
                    Limits::default(),
                )?;
            }
            let merged = engine.merge(args.base, args.left, args.right, MergeBudget::default())?;
            let receipt = repo.commit_source(
                &engine,
                merged.root,
                args.expected_generation,
                &args.request_key,
            )?;
            if merged.unresolved {
                eprintln!("Unresolved typed source; no marker text was substituted.");
            }
            receipt
        }
    };
    print_receipt(&receipt);
    Ok(())
}
fn relocate(repo: &mut Repository, args: Move, copy: bool) -> Result<Receipt> {
    let (base, mut engine) =
        mutation_base(repo, args.base, args.expected_generation, &args.request_key)?;
    let source = source_path(Some(&args.source))?;
    let destination = source_path(Some(&args.destination))?;
    let action = Relocate {
        expected_source: engine.read(base, &source)?,
        expected_destination: engine.read(base, &destination)?,
        source,
        destination,
        copy,
        overwrite: args.overwrite,
    };
    let root = engine.transform(base, &[action], &[])?;
    Ok(repo.commit_source(&engine, root, args.expected_generation, &args.request_key)?)
}
fn run(cli: CommandLine) -> Result<()> {
    let mutating = matches!(
        cli.command,
        Command::Init(_) | Command::Import(_) | Command::Source(_)
    );
    if mutating && !cli.experimental {
        return Err("writes require --experimental; keep independent backups (format and source custody are unqualified)".into());
    }
    if let Command::Init(_) = cli.command {
        Repository::init(&cli.repository)?;
        println!(
            "Initialized experimental native source store; no revision authored; project files untouched."
        );
        return Ok(());
    }
    let mut repo = Repository::open(&cli.repository)?;
    match cli.command {
        Command::Init(_) => unreachable!(),
        Command::Status(_) => {
            let Selection { root, generation } = repo.status()?;
            println!(
                "Experimental native source store\nRoot: {root}\nGeneration: {generation}\nFilesystem: not scanned\nCustody: unqualified; run verify for present closure integrity only"
            );
        }
        Command::Show(args) => {
            let root = selected(&repo, args.root)?;
            let engine = repo.load_root(root)?;
            let path = source_path(args.path.as_deref())?;
            let entry = engine.read(root, &path)?;
            match entry {
                Entry::File { blob, .. } | Entry::Symlink(blob) => {
                    let Object::Blob(bytes) = engine.object(blob)? else {
                        return Err("corrupt blob type".into());
                    };
                    std::io::stdout().lock().write_all(bytes)?;
                }
                Entry::Directory(_) => {
                    for (name, entry) in engine.list(root, &path)? {
                        println!("{}\t{}", name.escape_ascii(), entry_text(&entry));
                    }
                }
                Entry::Conflict(id) => {
                    let Object::Conflict(c) = engine.object(id)? else {
                        return Err("corrupt conflict type".into());
                    };
                    println!("Conflict {id}");
                    for (sign, terms) in [("+", &c.positive), ("-", &c.negative)] {
                        for (entry, count) in terms {
                            println!("{sign}{count} {}", entry_text(entry));
                        }
                    }
                }
                Entry::Absent => return Err("path is absent in exact source".into()),
            }
        }
        Command::Diff(args) => {
            let mut engine = repo.load_root(args.left)?;
            let other = repo.load_root(args.right)?;
            engine.import(
                other
                    .objects()
                    .map(|(id, o)| Ok((id, o.encode()?)))
                    .collect::<avc::Result<Vec<_>>>()?,
                args.right,
                Limits::default(),
            )?;
            for change in engine.diff(args.left, args.right)? {
                println!(
                    "{}\t{} -> {}",
                    display_path(&change.path),
                    entry_text(&change.before),
                    entry_text(&change.after)
                );
            }
        }
        Command::Verify(args) => {
            let root = selected(&repo, args.root)?;
            repo.load_root(root)?;
            println!(
                "Complete present closure verified: {root}\nNot a power-loss or source-custody qualification."
            );
        }
        Command::Export(args) => {
            let root = selected(&repo, args.root)?;
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&args.output)?;
            repo.export(root, &mut output)?;
            output.sync_all()?;
            File::open(
                args.output
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .sync_all()?;
            println!("Exported exact source closure {root}; private operations excluded.");
        }
        Command::Import(args) => {
            let receipt = repo.import(File::open(args.input)?, &args.request_key)?;
            print_receipt(&receipt);
        }
        Command::Receipt(args) => {
            let receipt = repo
                .receipt(&args.request_key)?
                .ok_or("request has no committed receipt; no success may be assumed")?;
            print_receipt(&receipt);
        }
        Command::Source(args) => source(&mut repo, args.command)?,
    }
    Ok(())
}
fn main() {
    if let Err(error) = run(CommandLine::parse()) {
        eprintln!("avc: {error}");
        std::process::exit(1);
    }
}
