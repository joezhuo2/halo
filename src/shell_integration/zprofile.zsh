ZDOTDIR="$HALO_ZDOTDIR"
[[ -f "$ZDOTDIR/.zprofile" ]] && builtin source "$ZDOTDIR/.zprofile"
HALO_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$HALO_OWN_ZDOTDIR"
