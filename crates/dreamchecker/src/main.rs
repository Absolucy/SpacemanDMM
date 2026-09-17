//! DreamChecker, a robust static analysis and typechecking engine for
//! DreamMaker.

use std::path::Path;

extern crate dreamchecker;
extern crate dreammaker as dm;
#[macro_use]
extern crate serde_json;

// ----------------------------------------------------------------------------
// Command-line interface

fn main() {
    // command-line args
    let mut environment = None;
    let mut config_file = None;
    let mut json = false;
    let mut parse_only = false;
    let mut sleep_verdicts = None;
    let mut unresolved_calls_sleep = true;

    let mut args = std::env::args();
    let _ = args.next(); // skip executable name
    while let Some(arg) = args.next() {
        if arg == "-V" || arg == "--version" {
            println!(
                "dreamchecker {}  Copyright (C) 2017-2025  Tad Hardesty",
                env!("CARGO_PKG_VERSION")
            );
            println!(
                "{}",
                include_str!(concat!(env!("OUT_DIR"), "/build-info.txt"))
            );
            println!("This program comes with ABSOLUTELY NO WARRANTY. This is free software,");
            println!("and you are welcome to redistribute it under the conditions of the GNU");
            println!("General Public License version 3.");
            return;
        } else if arg == "-e" {
            environment = Some(args.next().expect("must specify a value for -e"));
        } else if arg == "-c" {
            config_file = Some(args.next().expect("must specify a file for -c"));
        } else if arg == "--json" {
            json = true;
        } else if arg == "--parse-only" {
            parse_only = true;
        } else if arg == "--sleep-verdicts" {
            sleep_verdicts = Some(
                args.next()
                    .expect("must specify a file for --sleep-verdicts"),
            );
        } else if arg == "--assume-unresolved-calls-dont-sleep" {
            unresolved_calls_sleep = false;
        } else {
            eprintln!("unknown argument: {arg}");
            return;
        }
    }

    let mut context = dm::Context::default();
    context.set_print_severity(Some(dm::Severity::Info));
    let dme = match (config_file, environment) {
        (Some(toml), Some(dme)) => {
            _ = context.configure_from_toml(toml.as_ref());
            let dme = Path::new(&dme);
            dme.strip_prefix(".").unwrap_or(dme).to_owned()
        },
        (Some(toml), None) => {
            let r = context.configure_from_toml(toml.as_ref());
            context.unwrap(r)
        },
        (None, Some(dme)) => context.configure_from_dme(dme.as_ref()),
        (None, None) => {
            let r = context.configure_from_directory(".".as_ref());
            context.unwrap(r)
        },
    };

    // Version 2 follows every override a receiver's type allows, which is
    // too broad to leave anything worth compiling.
    let version = context.config().dreamchecker.sleep_analysis_version;
    if sleep_verdicts.is_some() && !(version == 1 || version >= 3) {
        eprintln!("--sleep-verdicts needs sleep_analysis_version 1 or 3, not {version}");
        std::process::exit(2);
    }

    println!("============================================================");
    println!("Parsing {}...\n", dme.display());
    let pp = context.unwrap(dm::Preprocessor::new(&context, dme));
    let mut parser = dm::Parser::new(&context, pp);
    parser.enable_procs();
    let (fatal_errored, tree) = parser.parse_object_tree_2();

    if !parse_only && !fatal_errored {
        match &sleep_verdicts {
            Some(out) => {
                let mut allowlist =
                    dreamchecker::run_cli_sleep_allowlist(&context, &tree, unresolved_calls_sleep)
                        .join("\n");
                allowlist.push('\n');
                std::fs::write(out, allowlist).expect("failed to write --sleep-verdicts file");
            },
            None => dreamchecker::run_cli(&context, &tree),
        }
    }

    println!("============================================================");
    let errors = context
        .errors()
        .iter()
        .filter(|each| each.severity() <= dm::Severity::Info)
        .count();
    println!("Found {errors} diagnostics");

    if json {
        serde_json::to_writer(std::io::stdout().lock(), &json! {{
            "hint": context.errors().iter().filter(|each| each.severity() == dm::Severity::Hint).count(),
            "info": context.errors().iter().filter(|each| each.severity() == dm::Severity::Info).count(),
            "warning": context.errors().iter().filter(|each| each.severity() == dm::Severity::Warning).count(),
            "error": context.errors().iter().filter(|each| each.severity() == dm::Severity::Error).count(),
        }}).unwrap();
    }

    std::process::exit(if errors > 0 { 1 } else { 0 });
}
