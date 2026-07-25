# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_aozora_flavored_markdown_global_optspecs
    string join \n encoding= strict color= v/verbose q/quiet format= h/help V/version
end

function __fish_aozora_flavored_markdown_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_aozora_flavored_markdown_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_aozora_flavored_markdown_using_subcommand
    set -l cmd (__fish_aozora_flavored_markdown_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -l encoding -d 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files' -r -f -a "utf8\t''
sjis\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -l color -d 'When to colorize diagnostics: auto (TTY-aware), always, or never' -r -f -a "auto\t''
always\t''
never\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -l format -d 'Diagnostic output format: human-readable lines, or stable JSON for tooling' -r -f -a "human\t'Graphical diagnostics (severity, code, message, source snippet) for humans'
json\t'A stable `aozora-md.diagnostics.v1` JSON envelope for tooling'"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -l strict -d 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -s v -l verbose -d 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -s q -l quiet -d 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -s V -l version -d 'Print version'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -f -a "render" -d 'Render the input to HTML on stdout'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -f -a "check" -d 'Parse the input and report diagnostics without rendering'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -f -a "completions" -d 'Generate a shell completion script on stdout'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -f -a "_man" -d 'Render the man page (roff) on stdout. Hidden; used by packaging'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -s o -l output -d 'Write HTML here instead of stdout. Use `-` for stdout' -r -F
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -l encoding -d 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files' -r -f -a "utf8\t''
sjis\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -l color -d 'When to colorize diagnostics: auto (TTY-aware), always, or never' -r -f -a "auto\t''
always\t''
never\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -l format -d 'Diagnostic output format: human-readable lines, or stable JSON for tooling' -r -f -a "human\t'Graphical diagnostics (severity, code, message, source snippet) for humans'
json\t'A stable `aozora-md.diagnostics.v1` JSON envelope for tooling'"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -l strict -d 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -s v -l verbose -d 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -s q -l quiet -d 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand render" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand check" -l encoding -d 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files' -r -f -a "utf8\t''
sjis\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand check" -l color -d 'When to colorize diagnostics: auto (TTY-aware), always, or never' -r -f -a "auto\t''
always\t''
never\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand check" -l format -d 'Diagnostic output format: human-readable lines, or stable JSON for tooling' -r -f -a "human\t'Graphical diagnostics (severity, code, message, source snippet) for humans'
json\t'A stable `aozora-md.diagnostics.v1` JSON envelope for tooling'"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand check" -l strict -d 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand check" -s v -l verbose -d 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand check" -s q -l quiet -d 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand check" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand completions" -l encoding -d 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files' -r -f -a "utf8\t''
sjis\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand completions" -l color -d 'When to colorize diagnostics: auto (TTY-aware), always, or never' -r -f -a "auto\t''
always\t''
never\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand completions" -l format -d 'Diagnostic output format: human-readable lines, or stable JSON for tooling' -r -f -a "human\t'Graphical diagnostics (severity, code, message, source snippet) for humans'
json\t'A stable `aozora-md.diagnostics.v1` JSON envelope for tooling'"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand completions" -l strict -d 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand completions" -s v -l verbose -d 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand completions" -s q -l quiet -d 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand completions" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand _man" -l encoding -d 'Input encoding. Defaults to UTF-8; use `sjis` for raw Aozora Bunko files' -r -f -a "utf8\t''
sjis\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand _man" -l color -d 'When to colorize diagnostics: auto (TTY-aware), always, or never' -r -f -a "auto\t''
always\t''
never\t''"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand _man" -l format -d 'Diagnostic output format: human-readable lines, or stable JSON for tooling' -r -f -a "human\t'Graphical diagnostics (severity, code, message, source snippet) for humans'
json\t'A stable `aozora-md.diagnostics.v1` JSON envelope for tooling'"
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand _man" -l strict -d 'Treat any lexer/parser diagnostic as a hard error (exit 2). Default: warn and pass through'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand _man" -s v -l verbose -d 'Increase log verbosity (-v info, -vv debug, -vvv trace). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand _man" -s q -l quiet -d 'Decrease log verbosity (-q errors only). `RUST_LOG` overrides'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand _man" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand help; and not __fish_seen_subcommand_from render check completions _man help" -f -a "render" -d 'Render the input to HTML on stdout'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand help; and not __fish_seen_subcommand_from render check completions _man help" -f -a "check" -d 'Parse the input and report diagnostics without rendering'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand help; and not __fish_seen_subcommand_from render check completions _man help" -f -a "completions" -d 'Generate a shell completion script on stdout'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand help; and not __fish_seen_subcommand_from render check completions _man help" -f -a "_man" -d 'Render the man page (roff) on stdout. Hidden; used by packaging'
complete -c aozora-flavored-markdown -n "__fish_aozora_flavored_markdown_using_subcommand help; and not __fish_seen_subcommand_from render check completions _man help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
