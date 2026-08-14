#!/bin/sh

set -eu

fail() {
    printf '%s\n' "ci-resource: $*" >&2
    exit 1
}

require_environment() {
    : "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
    : "${RUNNER_TEMP:?RUNNER_TEMP is required}"
}

canonical_non_root_directory() {
    directory_label=$1
    directory_path=$2
    case "${directory_path}" in
        /*) ;;
        *) fail "${directory_label} must be an absolute path: ${directory_path}" ;;
    esac
    test -d "${directory_path}" || fail "${directory_label} must be an existing directory: ${directory_path}"
    canonical_path=$(CDPATH= cd -P "${directory_path}" 2>/dev/null && pwd -P) \
        || fail "could not resolve ${directory_label}: ${directory_path}"
    test "${canonical_path}" != "/" || fail "${directory_label} must not resolve to the filesystem root"
    printf '%s\n' "${canonical_path}"
}

validate_resource_roots() {
    case "${runner_root}" in
        "${workspace_root}"|"${workspace_root}"/*)
            fail "RUNNER_TEMP must not resolve inside GITHUB_WORKSPACE"
            ;;
    esac
    case "${workspace_root}" in
        "${runner_root}"|"${runner_root}"/*)
            fail "GITHUB_WORKSPACE and RUNNER_TEMP must not contain one another"
            ;;
    esac
}

validate_managed_path() {
    managed_path=$1
    case "${managed_path}" in
        /*) ;;
        *) fail "managed path must be absolute: ${managed_path}" ;;
    esac
    child_name=${managed_path##*/}
    case "${child_name}" in
        ""|.|..)
            fail "managed path must have one non-special child name: ${managed_path}"
            ;;
    esac
    parent_path=${managed_path%/*}
    test -d "${parent_path}" \
        || fail "managed path parent must be an existing directory: ${managed_path}"
    canonical_parent=$(CDPATH= cd -P "${parent_path}" 2>/dev/null && pwd -P) \
        || fail "could not resolve managed path parent: ${managed_path}"
    test "${canonical_parent}" = "${runner_root}" \
        || fail "managed path must be a direct child of RUNNER_TEMP: ${managed_path}"
}

workspace_targets() {
    find "${workspace_root}" \
        -path "${workspace_root}/.git" -prune -o \
        \( -type d -o -type l \) -name target -print
}

assert_no_workspace_targets() {
    unexpected_targets=$(workspace_targets)
    if test -n "${unexpected_targets}"
    then
        printf '%s\n' "unexpected Cargo target directories exist inside the checkout:" >&2
        printf '%s\n' "${unexpected_targets}" >&2
        return 1
    fi
}

observe_paths() {
    for observed_path in "$@"
    do
        if test -e "${observed_path}" || test -L "${observed_path}"
        then
            du -sh "${observed_path}" || true
        fi
    done
    df -h "${RUNNER_TEMP}" || true
}

prepare() {
    test "$#" -ge 2 || fail "prepare requires MINIMUM_KIB and at least one managed path"
    minimum_kib=$1
    shift
    case "${minimum_kib}" in
        ""|*[!0-9]*) fail "minimum free space must be an integer KiB value" ;;
    esac
    test "${minimum_kib}" -gt 0 || fail "minimum free space must be positive"
    for managed_path in "$@"
    do
        validate_managed_path "${managed_path}"
    done
    assert_no_workspace_targets || fail "refusing to start with checkout-local build artifacts"

    available_kib=$(df -Pk "${RUNNER_TEMP}" | awk 'NR == 2 { print $4 }')
    test -n "${available_kib}" || fail "could not determine free space in RUNNER_TEMP"
    if test "${available_kib}" -lt "${minimum_kib}"
    then
        printf '%s\n' \
            "job requires ${minimum_kib} KiB free in RUNNER_TEMP; found ${available_kib} KiB" >&2
        df -h "${RUNNER_TEMP}" >&2
        exit 1
    fi

    for managed_path in "$@"
    do
        rm -rf -- "${managed_path}"
        mkdir -p "${managed_path}"
    done
    df -h "${RUNNER_TEMP}"
}

reset_paths() {
    test "$#" -ge 1 || fail "reset requires at least one managed path"
    for managed_path in "$@"
    do
        validate_managed_path "${managed_path}"
    done
    observe_paths "$@"
    for managed_path in "$@"
    do
        rm -rf -- "${managed_path}"
        mkdir -p "${managed_path}"
    done
}

forbidden_shims() {
    test "$#" -eq 1 || fail "forbidden-shims requires exactly one managed path"
    shim_dir=$1
    validate_managed_path "${shim_dir}"
    rm -rf -- "${shim_dir}"
    mkdir -p "${shim_dir}"
    cat > "${shim_dir}/fail" <<'EOF'
#!/bin/sh
printf '%s\n' "unexpected external tool invocation: $0" >&2
exit 86
EOF
    chmod 755 "${shim_dir}/fail"

    for tool in cmake cmake3 clang clang++ python python3 python-config python3-config pip pip3 pipx uv conda poetry pytest maturin hf huggingface-cli
    do
        ln -s fail "${shim_dir}/${tool}"
    done

    : "${GITHUB_ENV:?GITHUB_ENV is required for forbidden shims}"
    : "${GITHUB_PATH:?GITHUB_PATH is required for forbidden shims}"
    printf 'CMAKE=%s\n' "${shim_dir}/cmake" >> "${GITHUB_ENV}"
    printf 'PYTHON=%s\n' "${shim_dir}/python" >> "${GITHUB_ENV}"
    printf 'PYTHON3=%s\n' "${shim_dir}/python3" >> "${GITHUB_ENV}"
    printf '%s\n' "${shim_dir}" >> "${GITHUB_PATH}"
}

cleanup() {
    test "$#" -ge 1 || fail "cleanup requires at least one managed path"
    for managed_path in "$@"
    do
        validate_managed_path "${managed_path}"
    done

    failed=0
    observe_paths "$@"
    if ! assert_no_workspace_targets
    then
        failed=1
    fi

    for managed_path in "$@"
    do
        if ! rm -rf -- "${managed_path}"
        then
            printf '%s\n' "failed to remove managed path: ${managed_path}" >&2
            failed=1
        fi
    done
    for managed_path in "$@"
    do
        if test -e "${managed_path}" || test -L "${managed_path}"
        then
            printf '%s\n' "cleanup path still exists: ${managed_path}" >&2
            failed=1
        fi
    done
    if ! assert_no_workspace_targets
    then
        failed=1
    fi
    exit "${failed}"
}

require_environment
workspace_root=$(canonical_non_root_directory GITHUB_WORKSPACE "${GITHUB_WORKSPACE}")
runner_root=$(canonical_non_root_directory RUNNER_TEMP "${RUNNER_TEMP}")
validate_resource_roots
command_name=${1:-}
test -n "${command_name}" || fail "expected prepare, reset, forbidden-shims, or cleanup"
shift

case "${command_name}" in
    prepare) prepare "$@" ;;
    reset) reset_paths "$@" ;;
    forbidden-shims) forbidden_shims "$@" ;;
    cleanup) cleanup "$@" ;;
    *) fail "unknown command: ${command_name}" ;;
esac
