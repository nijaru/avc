//! Thin experimental CLI; source selection and authored history are explicit.
use avc::engine::{Edit, Engine, Limits, MergeBudget, Relocate};
use avc::repository::{Receipt, Repository, Selection};
use avc::{Entry, Id, Object};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use usage_rs::{Args, Cli, Subcommands};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Experimental native version control. Unreleased format; keep independent backups.
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
    Restore(Restore),
    Receipt(ReceiptArgs),
    Source(Source),
    Commit(Commit),
    /// Rewrite the selected tip using selected source, preserving ChangeId and parents.
    Amend(Commit),
    History(History),
    Log(Log),
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
/// Verify the selected source or an explicit source/revision owning closure.
#[derive(Args)]
struct Verify {
    #[usage(long)]
    root: Option<Id>,
}
/// Export exact source, or selected native history with --history.
#[derive(Args)]
struct Export {
    /// Include selected source, authored head and its complete native ancestry.
    #[usage(long, conflicts("--root"))]
    history: bool,
    #[usage(long)]
    root: Option<Id>,
    #[usage(long)]
    output: PathBuf,
}
/// Restore source (or --history) into an untouched initialized store.
#[derive(Args)]
struct Import {
    /// Require selected-history framing; never infer history from source-only input.
    #[usage(long)]
    history: bool,
    #[usage(long)]
    input: PathBuf,
    #[usage(long)]
    request_key: String,
}
/// Materialize exact source into an exclusively new directory (not a workspace).
#[derive(Args)]
struct Restore {
    #[usage(long)]
    root: Option<Id>,
    #[usage(long)]
    to: PathBuf,
}
/// Inspect an immutable request outcome after a lost response.
#[derive(Args)]
struct ReceiptArgs {
    request_key: String,
}
/// Author the selected source, not live filesystem files. Does not scan a workspace.
#[derive(Args)]
struct Commit {
    /// Author identity, supplied explicitly or through AVC_AUTHOR.
    #[usage(long, env = "AVC_AUTHOR")]
    author: String,
    #[usage(long, short = 'm')]
    message: String,
    #[usage(long)]
    expected_generation: u64,
    #[usage(long)]
    request_key: String,
}
/// Generation-checked local history operations; no filesystem or remote effects.
#[derive(Args)]
struct History {
    #[usage(subcommand)]
    command: HistoryCommand,
}
#[derive(Subcommands)]
enum HistoryCommand {
    /// Restore the exact current operation's before-source/head under a new generation.
    Undo(Undo),
}
#[derive(Args)]
struct Undo {
    /// Request key of the operation that still owns the current selection/generation.
    operation: String,
    #[usage(long)]
    expected_generation: u64,
    #[usage(long)]
    request_key: String,
}
/// Show newest-first first-parent history; source edits are not commits.
#[derive(Args)]
struct Log {
    #[usage(long)]
    revision: Option<Id>,
    #[usage(long, default = "20")]
    limit: usize,
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
    let revision = receipt
        .after
        .revision
        .map(|id| format!(" revision={id}"))
        .unwrap_or_default();
    let undo = match &receipt.operation {
        avc::repository::Operation::Undo { target } => format!(" undo={}", target.escape_debug()),
        _ => String::new(),
    };
    println!(
        "root={} generation={} request={}{}{}",
        receipt.after.root,
        receipt.after.generation,
        receipt.request_key.escape_debug(),
        revision,
        undo
    );
}
// Publication is already acknowledged before these optional comparisons run.
// Damaged comparison metadata must not hide a committed/recovered receipt.
fn amend_diagnostics(repo: &Repository, receipt: &Receipt) -> Result<()> {
    let before = repo.revision(
        receipt
            .before
            .revision
            .ok_or("amend receipt has no predecessor")?,
    )?;
    let after = repo.revision(
        receipt
            .after
            .revision
            .ok_or("amend receipt has no revision")?,
    )?;
    if before.root == after.root {
        eprintln!("Source unchanged from predecessor; a new immutable revision was authored.");
    }
    let empty = match after.parents.as_slice() {
        [] => after.root == Object::Map(avc::Node::Leaf(Vec::new())).id()?,
        [parent] => after.root == repo.revision(*parent)?.root,
        _ => false,
    };
    if empty {
        eprintln!("Empty change: source equals its comparison base; predecessor retained.");
    }
    Ok(())
}
fn mutation_base(
    repo: &Repository,
    base: Option<Id>,
    expected: u64,
    key: &str,
) -> Result<(Id, Engine)> {
    let previous = repo.preparation_base(expected, key)?;
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
        Command::Init(_)
            | Command::Import(_)
            | Command::Source(_)
            | Command::Restore(_)
            | Command::Commit(_)
            | Command::Amend(_)
            | Command::History(_)
    );
    if mutating && !cli.experimental {
        return Err(
            "writes require --experimental; the format is unreleased, so keep independent backups"
                .into(),
        );
    }
    if let Command::Init(_) = cli.command {
        Repository::init(&cli.repository)?;
        println!(
            "Initialized experimental native repository; no revision authored; project files untouched."
        );
        return Ok(());
    }
    let mut repo = Repository::open(&cli.repository)?;
    match cli.command {
        Command::Init(_) => unreachable!(),
        Command::Status(_) => {
            let Selection {
                root,
                generation,
                revision,
            } = repo.status()?;
            let revision = revision
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unborn".to_owned());
            println!(
                "Experimental native repository\nRoot: {root}\nRevision: {revision}\nGeneration: {generation}\nFilesystem: not scanned"
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
            println!("Complete present closure verified: {root}");
        }
        Command::Export(args) => {
            if args.history {
                let snapshot = repo.export_history_file(&args.output)?;
                let head = snapshot
                    .revision
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "unborn".to_owned());
                println!(
                    "Exported selected native history: root={} revision={head}; local operations, instance/workspace and Git metadata excluded.",
                    snapshot.root
                );
            } else {
                let root = selected(&repo, args.root)?;
                repo.export_file(root, &args.output)?;
                println!(
                    "Exported exact source closure {root}; authored history and private operations excluded."
                );
            }
        }
        Command::Import(args) => {
            let input = File::open(args.input)?;
            let receipt = if args.history {
                repo.import_history(input, &args.request_key)?
            } else {
                repo.import(input, &args.request_key)?
            };
            print_receipt(&receipt);
        }
        Command::Restore(args) => {
            let root = selected(&repo, args.root)?;
            let engine = repo.load_root(root)?;
            avc::materialize::restore(&engine,root,&args.to).map_err(|error|format!("{error}; source remains retained; destination may be incomplete—use a different fresh directory"))?;
            println!(
                "Materialized {root} into {}; no workspace registered.",
                args.to.display()
            );
        }
        Command::Receipt(args) => {
            let receipt = repo
                .receipt(&args.request_key)?
                .ok_or("request has no committed receipt; no success may be assumed")?;
            print_receipt(&receipt);
        }
        Command::Source(args) => source(&mut repo, args.command)?,
        Command::Commit(args) => {
            let receipt = repo.commit(
                &args.author,
                &args.message,
                args.expected_generation,
                &args.request_key,
            )?;
            print_receipt(&receipt);
        }
        Command::Amend(args) => {
            let receipt = repo.amend(
                &args.author,
                &args.message,
                args.expected_generation,
                &args.request_key,
            )?;
            print_receipt(&receipt);
            if let Err(error) = amend_diagnostics(&repo, &receipt) {
                eprintln!("Amend committed; comparison diagnostics unavailable: {error}");
            }
        }
        Command::History(args) => match args.command {
            HistoryCommand::Undo(args) => {
                let receipt =
                    repo.undo(&args.operation, args.expected_generation, &args.request_key)?;
                print_receipt(&receipt);
                eprintln!(
                    "Local source/head selection restored under a new generation; filesystem and remotes unchanged."
                );
            }
        },
        Command::Log(args) => {
            let records = repo.log(args.revision, args.limit)?;
            let mut output = std::io::stdout().lock();
            for (id, revision) in records {
                let change: String = revision
                    .change
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                writeln!(
                    output,
                    "Revision: {id}\nChange: {change}\nSource: {}\nAuthor: {}\nTime: {} ns UTC (offset {} minutes)",
                    revision.root,
                    revision.author.escape_debug(),
                    revision.time_ns,
                    revision.timezone_minutes
                )?;
                for parent in revision.parents {
                    writeln!(output, "Parent: {parent}")?;
                }
                writeln!(output, "Message: {}\n", revision.message.escape_debug())?;
            }
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run(CommandLine::parse()) {
        eprintln!("avc: {error}");
        std::process::exit(1);
    }
}
