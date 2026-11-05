# App-owned, launch-local hooks. add-zsh-hook preserves the user's existing hooks.
autoload -Uz add-zsh-hook
function __orlo_precmd() {
    local result=$?
    [[ $PROMPT == *$'\e]133;B\a'* ]] || PROMPT+=$'%{\e]133;B\a%}'
    printf '\e]133;D;%d\a\e]133;A\a' "$result"
    local LC_ALL=C orlo_pwd=$PWD encoded='' char hex
    local -i i
    for ((i=1; i<=${#orlo_pwd}; i++)); do
        char=$orlo_pwd[i]
        case $char in
            [a-zA-Z0-9/._~-]) encoded+=$char ;;
            *) printf -v hex '%%%02X' "'$char"; encoded+=$hex ;;
        esac
    done
    printf '\e]7;file://%s\a' "$encoded"
}
function __orlo_preexec() {
    printf '\e]133;C\a'
}
add-zsh-hook precmd __orlo_precmd
add-zsh-hook preexec __orlo_preexec
PROMPT+=$'%{\e]133;B\a%}'

# The widget admits the live ZLE buffer, independently of delayed terminal output.
autoload -Uz add-zle-hook-widget
function __orlo_line_init() {
    [[ -z $BUFFER ]] && printf '\e]133;OrloReady\a'
}
function __orlo_sync_folder() {
    local command
    IFS= builtin read -r command 2>/dev/null < "$ORLO_INTEGRATION_DIR/sync-folder" || command=''
    builtin : > "$ORLO_INTEGRATION_DIR/sync-folder"
    if [[ -z $command || -n $BUFFER || $PENDING != 0 || $KEYS_QUEUED_COUNT != 0 ]]; then
        printf '\e]133;OrloSyncRefused\a'
        return
    fi
    BUFFER=$command
    zle .accept-line
}
zle -N __orlo_sync_folder
bindkey -M emacs '\e[99;39~' __orlo_sync_folder
bindkey -M viins '\e[99;39~' __orlo_sync_folder
bindkey -M vicmd '\e[99;39~' __orlo_sync_folder
add-zle-hook-widget line-init __orlo_line_init
