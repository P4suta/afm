// Everything the binary does once clap has spoken: build the book, report
// what the chapters raised, and turn that into an exit code. `main.rs` is a
// shim over `run`, so the behaviour of the CLI is read here.

use std::process::ExitCode;

use aozora_flavored_markdown_epub::{BuildOptions, CheckOptions, build, check};
use clap::Parser;

use crate::args::{Cli, Cmd, DiagFormat};
use crate::output::emit_diagnostics;

pub(crate) fn run() -> ExitCode {
    match dispatch(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

/// # Errors
///
/// Propagates whatever the library raises while building.
pub(crate) fn dispatch(cli: Cli) -> miette::Result<ExitCode> {
    let report = match cli.cmd {
        Cmd::Build {
            input,
            metadata,
            output,
        } => build(&BuildOptions::new(&input, &metadata, &output))?,
        Cmd::Check { input, metadata } => check(&CheckOptions::new(&input, &metadata))?,
    };
    emit_diagnostics(&report, cli.format);
    if cli.strict && !report.is_empty() {
        // In JSON mode the envelope and the exit code carry the failure; a
        // free-form line would corrupt a stdout stream.
        if matches!(cli.format, DiagFormat::Human) {
            eprintln!(
                "{} 件の診断を報告しました (--strict)",
                report.diagnostic_count()
            );
        }
        return Ok(ExitCode::from(2));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::iter;
    use std::path::PathBuf;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(iter::once("aozora-flavored-markdown-epub").chain(args.iter().copied()))
    }

    #[test]
    fn parses_build_with_all_paths() {
        let cli = parse(&[
            "build",
            "--input",
            "m",
            "--metadata",
            "b.toml",
            "--output",
            "o.epub",
        ])
        .expect("parses");
        match cli.cmd {
            Cmd::Build {
                input,
                metadata,
                output,
            } => {
                assert_eq!(input, PathBuf::from("m"));
                assert_eq!(metadata, PathBuf::from("b.toml"));
                assert_eq!(output, PathBuf::from("o.epub"));
            }
            Cmd::Check { .. } => panic!("expected build"),
        }
    }

    #[test]
    fn output_has_a_short_flag() {
        let cli = parse(&[
            "build",
            "--input",
            "m",
            "--metadata",
            "b.toml",
            "-o",
            "o.epub",
        ])
        .expect("parses");
        match cli.cmd {
            Cmd::Build { output, .. } => assert_eq!(output, PathBuf::from("o.epub")),
            Cmd::Check { .. } => panic!("expected build"),
        }
    }

    #[test]
    fn build_requires_output() {
        parse(&["build", "--input", "m", "--metadata", "b.toml"]).unwrap_err();
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        parse(&["frobnicate"]).unwrap_err();
    }

    #[test]
    fn parses_non_writing_check() {
        let cli = parse(&["check", "--input", "m", "--metadata", "b.toml"]).unwrap();
        match cli.cmd {
            Cmd::Check { input, metadata } => {
                assert_eq!(input, PathBuf::from("m"));
                assert_eq!(metadata, PathBuf::from("b.toml"));
            }
            Cmd::Build { .. } => panic!("expected check"),
        }
    }

    #[test]
    fn run_builds_an_epub_from_a_manuscript_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manuscript = dir.path().join("manuscript");
        fs::create_dir(&manuscript).expect("mkdir");
        fs::write(manuscript.join("001-chapter.md"), "Hello").expect("write md");
        let metadata = dir.path().join("book.toml");
        fs::write(
            &metadata,
            "title = \"T\"\ncreator = \"A\"\nlanguage = \"ja\"\n",
        )
        .expect("write toml");
        let output = dir.path().join("out.epub");

        let cli = parse(&[
            "build",
            "--input",
            manuscript.to_str().expect("utf8 input"),
            "--metadata",
            metadata.to_str().expect("utf8 metadata"),
            "--output",
            output.to_str().expect("utf8 output"),
        ])
        .expect("parses");

        let _code = dispatch(cli).expect("build succeeds");
        assert!(output.exists(), "the .epub output must be written");
    }

    #[test]
    fn run_errors_on_missing_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("only.md"), "x").expect("write md");
        let missing = dir.path().join("does-not-exist.toml");
        let output = dir.path().join("out.epub");

        let cli = parse(&[
            "build",
            "--input",
            dir.path().join("only.md").to_str().expect("utf8 input"),
            "--metadata",
            missing.to_str().expect("utf8 metadata"),
            "--output",
            output.to_str().expect("utf8 output"),
        ])
        .expect("parses");

        dispatch(cli).unwrap_err();
    }

    #[test]
    fn check_does_not_write_an_epub() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manuscript = dir.path().join("manuscript");
        fs::create_dir(&manuscript).expect("mkdir");
        fs::write(manuscript.join("001-chapter.md"), "Hello").expect("write md");
        let metadata = dir.path().join("book.toml");
        fs::write(
            &metadata,
            "title = \"T\"\ncreator = \"A\"\nlanguage = \"ja\"\n",
        )
        .expect("write toml");

        let cli = parse(&[
            "check",
            "--input",
            manuscript.to_str().expect("utf8 input"),
            "--metadata",
            metadata.to_str().expect("utf8 metadata"),
        ])
        .expect("parses");
        assert_eq!(dispatch(cli).unwrap(), ExitCode::SUCCESS);
        assert!(
            fs::read_dir(dir.path()).unwrap().all(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "epub")),
            "check must not create an EPUB"
        );
    }
}
