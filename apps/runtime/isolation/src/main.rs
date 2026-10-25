//! `storm-runtime-isolation` — per-session isolation's root side
//! (decision 136).

use std::process::ExitCode;

use storm_runtime_isolation::{accounts, convert, operator, serve};

const USAGE: &str = "\
usage: storm-runtime-isolation <command>

  on       turn per-session isolation on for this host (root; storm-runtime stopped)
  off      turn it off and restore every workspace (root; storm-runtime stopped)
  status   show whether isolation is on
  serve    the helper service; started by its socket unit, never by hand

  child-clone is run by the helper, as a spawned session's user, never by hand
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = operator::Paths::installed();
    let mut stdout = std::io::stdout();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve"] => serve::run(&paths.config),
        ["child-clone", ..] => return storm_runtime_isolation::clone::main(&args[1..]),
        ["on"] => operator::on(
            &operator::Real,
            &accounts::System,
            &convert::Real,
            &paths,
            &mut stdout,
        ),
        ["off"] => operator::off(
            &operator::Real,
            &accounts::System,
            &convert::Real,
            &paths,
            &mut stdout,
        ),
        ["status"] => operator::status(&operator::Real, &paths, &mut stdout),
        ["--version"] => {
            println!("storm-runtime-isolation {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("storm-runtime-isolation: {e:#}");
            ExitCode::FAILURE
        }
    }
}
