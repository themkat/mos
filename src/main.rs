//! MOS is a toolkit for building applications that target the MOS 6502 CPU.

//#![deny(missing_docs)]

use std::path::Path;

use clap::{App, AppSettings, Arg, ArgMatches};
use fs_err as fs;

use crate::commands::*;
use crate::config::Config;
use crate::core::parser;
use crate::errors::MosResult;

/// Contains the available CLI commands and their associated logic
mod commands;
/// Configuration file handling
mod config;
/// Contains all main business logic
mod core;
/// Error handling
mod errors;

#[cfg(windows)]
/// A platform-specific newline.
pub const LINE_ENDING: &str = "\r\n";
#[cfg(not(windows))]
/// A platform-specific newline
pub const LINE_ENDING: &str = "\n";

#[cfg(test)]
pub fn enable_tracing<F: Fn(simple_logger::SimpleLogger) -> simple_logger::SimpleLogger>(
    customizer: F,
) {
    use simple_logger::*;
    let logger = SimpleLogger::new().with_level(log::LevelFilter::Off);
    let logger = customizer(logger);
    logger.init().unwrap();
}

#[cfg(test)]
pub fn enable_default_tracing() {
    use simple_logger::*;
    SimpleLogger::new()
        .with_level(log::LevelFilter::Off)
        .with_module_level("mos", log::LevelFilter::Trace)
        .init()
        .unwrap();
}

fn get_app() -> App<'static> {
    App::new("mos")
        .about("https://mos.datatra.sh")
        .version(git_version::git_version!())
        .global_setting(AppSettings::ColoredHelp)
        .global_setting(AppSettings::ArgRequiredElseHelp)
        .arg(
            Arg::new("verbose")
                .short('v')
                .multiple(true)
                .about("Sets the level of verbosity"),
        )
        .arg(
            Arg::new("no-color")
                .long("no-color")
                .about("Disables colorized output"),
        )
        .subcommand(build_app())
        .subcommand(format_app())
}

fn run(args: ArgMatches) -> MosResult<()> {
    let mos_toml = Path::new("mos.toml");
    let cfg = match mos_toml.exists() {
        true => {
            let toml = fs::read_to_string(mos_toml)?;
            Some(Config::from_toml(&toml)?)
        }
        false => None,
    };

    match args.subcommand() {
        Some(("build", args)) => build_command(args),
        Some(("format", args)) => format_command(args, cfg),
        _ => panic!("Unknown subcommand"),
    }
}

fn main() {
    #[cfg(windows)]
    ansi_term::enable_ansi_support().unwrap();

    let args = get_app().get_matches();

    loggerv::Logger::new()
        .verbosity(args.occurrences_of("v"))
        .colors(!args.is_present("no-color"))
        .module_path(false)
        .init()
        .unwrap();

    match run(args) {
        Ok(()) => (),
        Err(e) => {
            log::error!("{}", e.format(true));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;

    use crate::get_app;

    #[test]
    fn can_invoke_build() {
        let args = get_app().get_matches_from(vec!["mos", "build", "test.asm"]);
        assert_eq!(args.subcommand_name(), Some("build"));

        match args.subcommand() {
            Some(("build", args)) => {
                assert_eq!(args.values_of("input").unwrap().collect_vec(), ["test.asm"]);
            }
            _ => panic!(),
        }
    }

    /*#[test]
    fn can_invoke_format() {
        let args = get_app().get_matches_from(vec!["mos", "format", "test.asm"]);
        assert_eq!(args.subcommand_name(), Some("format"));

        match args.subcommand() {
            Some(("format", args)) => {
                assert_eq!(args.values_of("input").unwrap().collect_vec(), ["test.asm"]);
            }
            _ => panic!(),
        }
    }*/
}
