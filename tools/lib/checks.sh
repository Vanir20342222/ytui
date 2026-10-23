# Shared reporting for the opt-in acceptance runners. Source after setting up Bash.
check_count=0
check_failures=()

run_check() {
    local label="$1"
    shift
    check_count=$((check_count + 1))
    printf '\nChecking %s\n' "$label"
    if "$@"; then
        printf 'PASS %s\n' "$label"
    else
        check_failures+=("$label")
        printf 'FAIL %s\n' "$label" >&2
    fi
}

finish_checks() {
    printf '\nCompleted %d check groups; %d failed.\n' "$check_count" "${#check_failures[@]}"
    if ((${#check_failures[@]})); then
        printf 'Failed: %s\n' "${check_failures[@]}"
        return 1
    fi
    return 0
}
