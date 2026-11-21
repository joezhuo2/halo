dir=$(dirname "$0")
shell=${SHELL:-$(getent passwd "$(id -un)" | cut -d: -f7)}
case "$shell" in
    */zsh) HALO_ZDOTDIR="${ZDOTDIR:-$HOME}" ZDOTDIR="$dir/zsh" exec "$shell" -l ;;
    */bash) HALO_SHELL_LOGIN=1 exec "$shell" --init-file "$dir/halo.bash" -i ;;
    *) exec "${shell:-sh}" -l ;;
esac
