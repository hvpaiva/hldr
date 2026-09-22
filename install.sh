#!/usr/bin/env bash
#
# Installs hldr, the command-line client of hvpaiva.dev:
#
#   curl -fsSL https://hvpaiva.dev/install.sh | bash
#   curl -fsSL https://hvpaiva.dev/install.sh | bash -s -- --version 0.1.0
#
# Each release attaches this script with its own version stamped in, so it
# installs the binary of that release unless told otherwise; unstamped, as
# in a checkout, it asks GitHub for the latest release. The binary's SHA-256
# is always checked, and its build provenance too when the GitHub CLI is
# installed and logged in.
#
# Options, or their environment variables:
#   --version X.Y.Z         HLDR_INSTALL_VERSION   the release to install
#   --dir DIR               HLDR_INSTALL_DIR       where hldr goes; ~/.local/bin
#   --require-attestation                          fail unless gh verifies it
#
# Everything runs from main, called on the last line, so a download cut
# short runs nothing.

set -u

readonly repo=hvpaiva/hldr
readonly stamped='@HLDR_VERSION@'
readonly semver='^[0-9]+\.[0-9]+\.[0-9]+$'

say() {
	printf 'hldr install: %s\n' "$*" >&2
}

die() {
	say "$*"
	exit 1
}

fetch() {
	curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$2" "$1"
}

usage() {
	cat >&2 <<-EOF
		usage: install.sh [--version X.Y.Z] [--dir DIR] [--require-attestation]
	EOF
}

# The release the latest tag points at, read from GitHub's redirect rather
# than its API, which limits anonymous callers to 60 requests an hour.
latest() {
	local url
	url=$(curl --proto '=https' --tlsv1.2 -fsSLI -o /dev/null -w '%{url_effective}' \
		"https://github.com/$repo/releases/latest") || return 1
	printf '%s\n' "${url##*/v}"
}

target() {
	local os arch
	os=$(uname -s)
	arch=$(uname -m)
	case "$os/$arch" in
	Linux/x86_64 | Linux/amd64) printf 'x86_64-unknown-linux-musl\n' ;;
	*) return 1 ;;
	esac
}

sha256() {
	local line
	if command -v sha256sum >/dev/null; then
		line=$(sha256sum "$1") || return 1
	else
		line=$(shasum -a 256 "$1") || return 1
	fi
	printf '%s\n' "${line%% *}"
}

# Checks the provenance GitHub recorded when the release workflow built the
# binary; every release carries one, so a missing one fails like a wrong
# one, as whoever swapped a binary and its checksum would leave. Returns 2
# when it cannot be checked here.
attested() {
	command -v gh >/dev/null || return 2
	gh auth status >/dev/null 2>&1 || return 2
	local report
	report=$(gh attestation verify "$1" --repo "$repo" 2>&1) || {
		printf '%s\n' "$report" >&2
		return 1
	}
}

installed() {
	[[ -x $1 ]] || return 1
	"$1" --plain version --client 2>/dev/null | {
		read -r _ _ version _
		printf '%s\n' "$version"
	}
}

main() {
	local version=${HLDR_INSTALL_VERSION:-}
	local dir=${HLDR_INSTALL_DIR:-}
	local require_attestation=false
	while (($# > 0)); do
		case $1 in
		--version)
			(($# >= 2)) || die "--version needs a value"
			version=$2
			shift 2
			;;
		--version=*)
			version=${1#*=}
			shift
			;;
		--dir)
			(($# >= 2)) || die "--dir needs a value"
			dir=$2
			shift 2
			;;
		--dir=*)
			dir=${1#*=}
			shift
			;;
		--require-attestation)
			require_attestation=true
			shift
			;;
		-h | --help)
			usage
			return 0
			;;
		*)
			usage
			die "unknown argument: $1"
			;;
		esac
	done

	command -v curl >/dev/null || die "curl is required"
	command -v sha256sum >/dev/null || command -v shasum >/dev/null ||
		die "sha256sum or shasum is required"
	local triple
	triple=$(target) || die "no build for $(uname -s) $(uname -m); releases ship Linux x86_64 only"

	if [[ -z $version ]]; then
		if [[ $stamped =~ $semver ]]; then
			version=$stamped
		else
			version=$(latest) || die "cannot find the latest release of $repo"
		fi
	fi
	version=${version#v}
	[[ $version =~ $semver ]] || die "not a version: $version"

	if [[ -z $dir ]]; then
		[[ -n ${HOME:-} ]] || die "HOME is not set; pass --dir"
		dir=$HOME/.local/bin
	fi
	local bin=$dir/hldr
	if [[ $(installed "$bin") == "$version" ]]; then
		say "hldr $version is already installed at $bin"
		return 0
	fi

	local tmp
	tmp=$(mktemp -d) || die "cannot create a temporary directory"
	# shellcheck disable=SC2064 # expand now: tmp is local to main
	trap "rm -rf -- '$tmp'" EXIT

	local asset=hldr-$triple
	local base=https://github.com/$repo/releases/download/v$version
	say "downloading hldr $version for $triple"
	fetch "$base/$asset" "$tmp/$asset" || die "cannot download $base/$asset"
	fetch "$base/$asset.sha256" "$tmp/$asset.sha256" || die "cannot download $base/$asset.sha256"

	local expected name actual
	read -r expected name <"$tmp/$asset.sha256" || die "$asset.sha256 is empty"
	[[ $name == "$asset" ]] || die "$asset.sha256 names $name, not $asset"
	actual=$(sha256 "$tmp/$asset") || die "cannot checksum $asset"
	[[ $actual == "$expected" ]] || die "checksum mismatch for $asset: expected $expected, got $actual"
	say "checksum verified"

	attested "$tmp/$asset"
	case $? in
	0) say "build provenance verified" ;;
	2)
		$require_attestation && die "cannot verify build provenance: install gh and run gh auth login"
		say "build provenance not checked: needs gh, logged in"
		;;
	*) die "build provenance does not verify for $asset" ;;
	esac

	mkdir -p -- "$dir" || die "cannot create $dir"
	local staged=$dir/.hldr.$$
	install -m 755 "$tmp/$asset" "$staged" || die "cannot write to $dir"
	mv -f -- "$staged" "$bin" || {
		rm -f -- "$staged"
		die "cannot replace $bin"
	}
	[[ $(installed "$bin") == "$version" ]] || die "$bin does not report version $version"
	say "installed hldr $version at $bin"

	case ":${PATH:-}:" in
	*":$dir:"*) ;;
	*) say "$dir is not on PATH; add it to run hldr by name" ;;
	esac
	say "next: set server: in ~/.config/hldr/config.yaml, then run hldr auth login to write"
	say "      (see https://github.com/$repo#use)"
}

main "$@"
