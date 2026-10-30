use std::{env, process::ExitCode};

const NOT_IMPLEMENTED: &str = "The native AVC engine is not implemented.\n\
The legacy Git-wrapper commands have been retired; this executable cannot operate on repositories.\n\
See docs/design.md, docs/ux.md and docs/roadmap.md for the proposed design and qualification work.";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let first = args.next();
    let only_one = args.next().is_none();

    if only_one
        && first
            .as_ref()
            .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "AVC {} — experimental native version control\n\n{}\n\nOptions:\n  -h, --help     Show this information\n  -V, --version  Show the package version",
            env!("CARGO_PKG_VERSION"),
            NOT_IMPLEMENTED
        );
        ExitCode::SUCCESS
    } else if only_one
        && first
            .as_ref()
            .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        println!("avc {}", env!("CARGO_PKG_VERSION"));
        ExitCode::SUCCESS
    } else {
        eprintln!("{NOT_IMPLEMENTED}");
        ExitCode::FAILURE
    }
}
