//! `aozora-flavored-markdown-epub` CLI — thin clap wrapper over the
//! `aozora_flavored_markdown_epub` library.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "aozora-flavored-markdown-epub",
    version,
    about,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Build an EPUB3 from a manuscript directory.
    Build {
        /// Input directory or single Aozora Flavored Markdown file.
        #[arg(long)]
        input: PathBuf,
        /// `book.toml` metadata path.
        #[arg(long)]
        metadata: PathBuf,
        /// Output `.epub` path.
        #[arg(long, short = 'o')]
        output: PathBuf,
    },
}

fn main() -> miette::Result<()> {
    run(Cli::parse())
}

/// # Errors
///
/// Propagates whatever the library raises while building.
fn run(cli: Cli) -> miette::Result<()> {
    match cli.cmd {
        Cmd::Build {
            input,
            metadata,
            output,
        } => aozora_flavored_markdown_epub::build(&aozora_flavored_markdown_epub::BuildOptions {
            input: &input,
            metadata: &metadata,
            output: &output,
        })?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::iter;

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

        run(cli).unwrap();
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

        assert!(run(cli).is_err());
    }
}
