
use builtin;
use str;

set edit:completion:arg-completer[aozora-flavored-markdown] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'aozora-flavored-markdown'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'aozora-flavored-markdown'= {
            cand --encoding 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files'
            cand --color 'When to colorize diagnostics: auto (TTY-aware), always, or never'
            cand --format 'Diagnostic output format: human-readable lines, or stable JSON for tooling'
            cand --strict 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
            cand -v 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand --verbose 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand -q 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand --quiet 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand -V 'Print version'
            cand --version 'Print version'
            cand render 'Render the input to HTML on stdout'
            cand check 'Parse the input and report diagnostics without rendering'
            cand fmt 'Canonicalize source and check, display, or write the result'
            cand completions 'Generate a shell completion script on stdout'
            cand _man 'Render the man page (roff) on stdout. Hidden; used by packaging'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'aozora-flavored-markdown;render'= {
            cand -o 'Write HTML here instead of stdout. Use `-` for stdout'
            cand --output 'Write HTML here instead of stdout. Use `-` for stdout'
            cand --encoding 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files'
            cand --color 'When to colorize diagnostics: auto (TTY-aware), always, or never'
            cand --format 'Diagnostic output format: human-readable lines, or stable JSON for tooling'
            cand --strict 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
            cand -v 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand --verbose 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand -q 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand --quiet 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'aozora-flavored-markdown;check'= {
            cand --encoding 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files'
            cand --color 'When to colorize diagnostics: auto (TTY-aware), always, or never'
            cand --format 'Diagnostic output format: human-readable lines, or stable JSON for tooling'
            cand --strict 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
            cand -v 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand --verbose 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand -q 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand --quiet 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'aozora-flavored-markdown;fmt'= {
            cand --encoding 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files'
            cand --color 'When to colorize diagnostics: auto (TTY-aware), always, or never'
            cand --format 'Diagnostic output format: human-readable lines, or stable JSON for tooling'
            cand --check 'Exit 1 when the source is not canonical; write nothing'
            cand --diff 'Print a unified diff; exit 1 when the source is not canonical'
            cand --write 'Replace the input file with canonical source'
            cand --strict 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
            cand -v 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand --verbose 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand -q 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand --quiet 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'aozora-flavored-markdown;completions'= {
            cand --encoding 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files'
            cand --color 'When to colorize diagnostics: auto (TTY-aware), always, or never'
            cand --format 'Diagnostic output format: human-readable lines, or stable JSON for tooling'
            cand --strict 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
            cand -v 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand --verbose 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand -q 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand --quiet 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'aozora-flavored-markdown;_man'= {
            cand --encoding 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files'
            cand --color 'When to colorize diagnostics: auto (TTY-aware), always, or never'
            cand --format 'Diagnostic output format: human-readable lines, or stable JSON for tooling'
            cand --strict 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
            cand -v 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand --verbose 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
            cand -q 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand --quiet 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'aozora-flavored-markdown;help'= {
            cand render 'Render the input to HTML on stdout'
            cand check 'Parse the input and report diagnostics without rendering'
            cand fmt 'Canonicalize source and check, display, or write the result'
            cand completions 'Generate a shell completion script on stdout'
            cand _man 'Render the man page (roff) on stdout. Hidden; used by packaging'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'aozora-flavored-markdown;help;render'= {
        }
        &'aozora-flavored-markdown;help;check'= {
        }
        &'aozora-flavored-markdown;help;fmt'= {
        }
        &'aozora-flavored-markdown;help;completions'= {
        }
        &'aozora-flavored-markdown;help;_man'= {
        }
        &'aozora-flavored-markdown;help;help'= {
        }
    ]
    $completions[$command]
}
