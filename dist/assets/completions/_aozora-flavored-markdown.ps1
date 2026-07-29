
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'aozora-flavored-markdown' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'aozora-flavored-markdown'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'aozora-flavored-markdown' {
            [CompletionResult]::new('--encoding', '--encoding', [CompletionResultType]::ParameterName, 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files')
            [CompletionResult]::new('--color', '--color', [CompletionResultType]::ParameterName, 'When to colorize diagnostics: auto (TTY-aware), always, or never')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Diagnostic output format: human-readable lines, or stable JSON for tooling')
            [CompletionResult]::new('--strict', '--strict', [CompletionResultType]::ParameterName, 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('-q', '-q', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('--quiet', '--quiet', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('render', 'render', [CompletionResultType]::ParameterValue, 'Render the input to HTML on stdout')
            [CompletionResult]::new('check', 'check', [CompletionResultType]::ParameterValue, 'Parse the input and report diagnostics without rendering')
            [CompletionResult]::new('fmt', 'fmt', [CompletionResultType]::ParameterValue, 'Canonicalize source and check, display, or write the result')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Generate a shell completion script on stdout')
            [CompletionResult]::new('_man', '_man', [CompletionResultType]::ParameterValue, 'Render the man page (roff) on stdout. Hidden; used by packaging')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'aozora-flavored-markdown;render' {
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, 'Write HTML here instead of stdout. Use `-` for stdout')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, 'Write HTML here instead of stdout. Use `-` for stdout')
            [CompletionResult]::new('--encoding', '--encoding', [CompletionResultType]::ParameterName, 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files')
            [CompletionResult]::new('--color', '--color', [CompletionResultType]::ParameterName, 'When to colorize diagnostics: auto (TTY-aware), always, or never')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Diagnostic output format: human-readable lines, or stable JSON for tooling')
            [CompletionResult]::new('--strict', '--strict', [CompletionResultType]::ParameterName, 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('-q', '-q', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('--quiet', '--quiet', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'aozora-flavored-markdown;check' {
            [CompletionResult]::new('--encoding', '--encoding', [CompletionResultType]::ParameterName, 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files')
            [CompletionResult]::new('--color', '--color', [CompletionResultType]::ParameterName, 'When to colorize diagnostics: auto (TTY-aware), always, or never')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Diagnostic output format: human-readable lines, or stable JSON for tooling')
            [CompletionResult]::new('--strict', '--strict', [CompletionResultType]::ParameterName, 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('-q', '-q', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('--quiet', '--quiet', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'aozora-flavored-markdown;fmt' {
            [CompletionResult]::new('--encoding', '--encoding', [CompletionResultType]::ParameterName, 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files')
            [CompletionResult]::new('--color', '--color', [CompletionResultType]::ParameterName, 'When to colorize diagnostics: auto (TTY-aware), always, or never')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Diagnostic output format: human-readable lines, or stable JSON for tooling')
            [CompletionResult]::new('--check', '--check', [CompletionResultType]::ParameterName, 'Exit 1 when the source is not canonical; write nothing')
            [CompletionResult]::new('--diff', '--diff', [CompletionResultType]::ParameterName, 'Print a unified diff; exit 1 when the source is not canonical')
            [CompletionResult]::new('--write', '--write', [CompletionResultType]::ParameterName, 'Replace the input file with canonical source')
            [CompletionResult]::new('--strict', '--strict', [CompletionResultType]::ParameterName, 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('-q', '-q', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('--quiet', '--quiet', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'aozora-flavored-markdown;completions' {
            [CompletionResult]::new('--encoding', '--encoding', [CompletionResultType]::ParameterName, 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files')
            [CompletionResult]::new('--color', '--color', [CompletionResultType]::ParameterName, 'When to colorize diagnostics: auto (TTY-aware), always, or never')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Diagnostic output format: human-readable lines, or stable JSON for tooling')
            [CompletionResult]::new('--strict', '--strict', [CompletionResultType]::ParameterName, 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('-q', '-q', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('--quiet', '--quiet', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'aozora-flavored-markdown;_man' {
            [CompletionResult]::new('--encoding', '--encoding', [CompletionResultType]::ParameterName, 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files')
            [CompletionResult]::new('--color', '--color', [CompletionResultType]::ParameterName, 'When to colorize diagnostics: auto (TTY-aware), always, or never')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Diagnostic output format: human-readable lines, or stable JSON for tooling')
            [CompletionResult]::new('--strict', '--strict', [CompletionResultType]::ParameterName, 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides')
            [CompletionResult]::new('-q', '-q', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('--quiet', '--quiet', [CompletionResultType]::ParameterName, 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'aozora-flavored-markdown;help' {
            [CompletionResult]::new('render', 'render', [CompletionResultType]::ParameterValue, 'Render the input to HTML on stdout')
            [CompletionResult]::new('check', 'check', [CompletionResultType]::ParameterValue, 'Parse the input and report diagnostics without rendering')
            [CompletionResult]::new('fmt', 'fmt', [CompletionResultType]::ParameterValue, 'Canonicalize source and check, display, or write the result')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Generate a shell completion script on stdout')
            [CompletionResult]::new('_man', '_man', [CompletionResultType]::ParameterValue, 'Render the man page (roff) on stdout. Hidden; used by packaging')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'aozora-flavored-markdown;help;render' {
            break
        }
        'aozora-flavored-markdown;help;check' {
            break
        }
        'aozora-flavored-markdown;help;fmt' {
            break
        }
        'aozora-flavored-markdown;help;completions' {
            break
        }
        'aozora-flavored-markdown;help;_man' {
            break
        }
        'aozora-flavored-markdown;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
