if [ -n "$HALO_SHELL_LOGIN" ]; then
    builtin unset HALO_SHELL_LOGIN
    [ -r /etc/profile ] && builtin . /etc/profile
    if [ -r ~/.bash_profile ]; then builtin . ~/.bash_profile
    elif [ -r ~/.bash_login ]; then builtin . ~/.bash_login
    elif [ -r ~/.profile ]; then builtin . ~/.profile
    fi
else
    [ -r ~/.bashrc ] && builtin . ~/.bashrc
fi

__halo_escape() {
    local s="${1//\\/\\\\}"
    s="${s//;/\\x3b}"
    s="${s//$'\n'/\\x0a}"
    builtin printf '%s' "$s"
}

__halo_ready=0
__halo_ran=0
__halo_status=0

__halo_preexec() {
    [ "$__halo_ready" = 1 ] || return
    [ -n "$COMP_LINE" ] && return
    __halo_ready=0
    __halo_ran=1
    local line
    line=$(HISTTIMEFORMAT= builtin history 1)
    line="${line#*[0-9]  }"
    builtin printf '\e]633;E;%s\a\e]633;C\a' "$(__halo_escape "${line:-$BASH_COMMAND}")"
}

__halo_save_status() {
    __halo_status=$?
}

__halo_precmd() {
    if [ "$__halo_ran" = 1 ]; then
        builtin printf '\e]633;D;%s\a' "$__halo_status"
    else
        builtin printf '\e]633;D\a'
    fi
    __halo_ran=0
    local dir="$PWD"
    case "$OSTYPE" in msys* | cygwin*) dir=$(builtin pwd -W) ;; esac
    builtin printf '\e]633;P;Cwd=%s\a' "$(__halo_escape "$dir")"
    if [[ "$PS1" != *'633;A'* ]]; then
        PS1='\[\e]633;A\a\]'"$PS1"'\[\e]633;B\a\]'
    fi
    __halo_ready=1
}

trap '__halo_preexec' DEBUG
__halo_nl=$'\n'
PROMPT_COMMAND="__halo_save_status${__halo_nl}${PROMPT_COMMAND:+$PROMPT_COMMAND$__halo_nl}__halo_precmd"
