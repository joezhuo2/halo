ZDOTDIR="$HALO_ZDOTDIR"
[[ -f "$ZDOTDIR/.zshrc" ]] && builtin source "$ZDOTDIR/.zshrc"
HALO_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$HALO_OWN_ZDOTDIR"

__halo_escape() {
    local s="${1//\\/\\\\}"
    s="${s//;/\\x3b}"
    s="${s//$'\n'/\\x0a}"
    builtin print -rn -- "$s"
}

__halo_ran=0
__halo_status=0

__halo_save_status() {
    __halo_status=$?
}

__halo_precmd() {
    if (( __halo_ran )); then
        builtin print -rn -- $'\e]633;D;'"$__halo_status"$'\a'
    else
        builtin print -rn -- $'\e]633;D\a'
    fi
    __halo_ran=0
    builtin print -rn -- $'\e]633;P;Cwd='"$(__halo_escape "$PWD")"$'\a'
    if [[ "$PS1" != *'633;A'* ]]; then
        PS1=$'%{\e]633;A\a%}'"$PS1"$'%{\e]633;B\a%}'
    fi
}

__halo_preexec() {
    __halo_ran=1
    builtin print -rn -- $'\e]633;E;'"$(__halo_escape "$1")"$'\a\e]633;C\a'
}

precmd_functions=(__halo_save_status $precmd_functions __halo_precmd)
preexec_functions+=(__halo_preexec)

[[ -o login ]] || ZDOTDIR="$HALO_ZDOTDIR"
