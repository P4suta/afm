_aozora-flavored-markdown() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="aozora__flavored__markdown"
                ;;
            aozora__flavored__markdown,_man)
                cmd="aozora__flavored__markdown__subcmd___man"
                ;;
            aozora__flavored__markdown,check)
                cmd="aozora__flavored__markdown__subcmd__check"
                ;;
            aozora__flavored__markdown,completions)
                cmd="aozora__flavored__markdown__subcmd__completions"
                ;;
            aozora__flavored__markdown,fmt)
                cmd="aozora__flavored__markdown__subcmd__fmt"
                ;;
            aozora__flavored__markdown,help)
                cmd="aozora__flavored__markdown__subcmd__help"
                ;;
            aozora__flavored__markdown,render)
                cmd="aozora__flavored__markdown__subcmd__render"
                ;;
            aozora__flavored__markdown__subcmd__help,_man)
                cmd="aozora__flavored__markdown__subcmd__help__subcmd___man"
                ;;
            aozora__flavored__markdown__subcmd__help,check)
                cmd="aozora__flavored__markdown__subcmd__help__subcmd__check"
                ;;
            aozora__flavored__markdown__subcmd__help,completions)
                cmd="aozora__flavored__markdown__subcmd__help__subcmd__completions"
                ;;
            aozora__flavored__markdown__subcmd__help,fmt)
                cmd="aozora__flavored__markdown__subcmd__help__subcmd__fmt"
                ;;
            aozora__flavored__markdown__subcmd__help,help)
                cmd="aozora__flavored__markdown__subcmd__help__subcmd__help"
                ;;
            aozora__flavored__markdown__subcmd__help,render)
                cmd="aozora__flavored__markdown__subcmd__help__subcmd__render"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        aozora__flavored__markdown)
            opts="-v -q -h -V --encoding --strict --color --verbose --quiet --format --help --version render check fmt completions _man help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --encoding)
                    COMPREPLY=($(compgen -W "utf8 sjis" -- "${cur}"))
                    return 0
                    ;;
                --color)
                    COMPREPLY=($(compgen -W "auto always never" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "human json" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd___man)
            opts="-v -q -h --encoding --strict --color --verbose --quiet --format --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --encoding)
                    COMPREPLY=($(compgen -W "utf8 sjis" -- "${cur}"))
                    return 0
                    ;;
                --color)
                    COMPREPLY=($(compgen -W "auto always never" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "human json" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__check)
            opts="-v -q -h --encoding --strict --color --verbose --quiet --format --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --encoding)
                    COMPREPLY=($(compgen -W "utf8 sjis" -- "${cur}"))
                    return 0
                    ;;
                --color)
                    COMPREPLY=($(compgen -W "auto always never" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "human json" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__completions)
            opts="-v -q -h --encoding --strict --color --verbose --quiet --format --help bash elvish fish powershell zsh"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --encoding)
                    COMPREPLY=($(compgen -W "utf8 sjis" -- "${cur}"))
                    return 0
                    ;;
                --color)
                    COMPREPLY=($(compgen -W "auto always never" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "human json" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__fmt)
            opts="-v -q -h --check --diff --write --encoding --strict --color --verbose --quiet --format --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --encoding)
                    COMPREPLY=($(compgen -W "utf8 sjis" -- "${cur}"))
                    return 0
                    ;;
                --color)
                    COMPREPLY=($(compgen -W "auto always never" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "human json" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__help)
            opts="render check fmt completions _man help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__help__subcmd___man)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__help__subcmd__check)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__help__subcmd__completions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__help__subcmd__fmt)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__help__subcmd__render)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        aozora__subcmd__flavored__subcmd__markdown__subcmd__render)
            opts="-o -v -q -h --output --encoding --strict --color --verbose --quiet --format --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --encoding)
                    COMPREPLY=($(compgen -W "utf8 sjis" -- "${cur}"))
                    return 0
                    ;;
                --color)
                    COMPREPLY=($(compgen -W "auto always never" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "human json" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _aozora-flavored-markdown -o nosort -o bashdefault -o default aozora-flavored-markdown
else
    complete -F _aozora-flavored-markdown -o bashdefault -o default aozora-flavored-markdown
fi
