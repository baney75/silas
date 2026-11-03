#!/usr/bin/env bash
# Silas Local Signing: a self-signed code-signing identity that lives only on this Mac,
# modeled on BaneySpaces Local Signing. Every build signed with it has the same designated
# requirement, expected to read
#   identifier "com.baney.silas" and certificate leaf = H"<SHA-1 of the certificate>"
# so macOS keeps Microphone and Accessibility across Mac-built releases without an Apple
# Developer ID. CI can't use it: the private key never leaves this Mac. See docs/signing.md.
#
#   scripts/signing-identity.sh ensure       create or restore the identity, unlock it, print its SHA-1
#   scripts/signing-identity.sh requirement  print the designated requirement it should give
#
# Everything lives in ~/Library/Application Support/Silas/Signing (SILAS_SIGNING_DIR overrides):
#   silas-signing.keychain-db  the keychain that holds the identity
#   keychain-password          random, 0600: the keychain's password
#   kept/identity.p12          the identity, kept so a lost keychain comes back with the same leaf
#   kept/identity-password     random, 0600: the .p12's password
# Passwords travel through files and stdin only, never a command's arguments, so `ps` never
# shows them. Back up kept/ with the rest of the Mac; losing it means a new requirement.
set -euo pipefail

name="Silas Local Signing"
dir="${SILAS_SIGNING_DIR:-$HOME/Library/Application Support/Silas/Signing}"
kc="$dir/silas-signing.keychain-db"
kc_pw="$dir/keychain-password"
kept="$dir/kept"
openssl="${OPENSSL:-/usr/bin/openssl}" # LibreSSL on macOS; its .p12 files import into the keychain
fail() { echo "signing-identity.sh: $*" >&2; exit 1; }
log() { echo "signing-identity.sh: $*" >&2; }

[ "$(uname)" = Darwin ] || fail "Silas Local Signing needs macOS"

pw() { tr -d '\n' < "$1"; }
new_password() { "$openssl" rand -hex 32 > "$1"; chmod 600 "$1"; }
# One `security` command fed on stdin (`security -i`), so a password in it stays off argv.
secure() { printf '%s\n' "$1" | security -i > /dev/null; }

identity_hash() {
  [ -f "$kc" ] || return 0
  security find-identity -p codesigning "$kc" 2>/dev/null | awk -v n="\"$name\"" 'index($0, n) {print $2; exit}'
}

make_identity() {
  local tmp legacy=""
  tmp=$(mktemp -d)
  cat > "$tmp/req.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF
  "$openssl" req -x509 -newkey rsa:2048 -nodes -days 7300 -config "$tmp/req.cnf" \
    -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null || { rm -rf "$tmp"; fail "openssl could not make the certificate"; }
  new_password "$kept/identity-password"
  # OpenSSL 3 encrypts .p12 files with ciphers the macOS keychain can't read.
  "$openssl" version | grep -q '^OpenSSL 3' && legacy=-legacy
  "$openssl" pkcs12 -export $legacy -inkey "$tmp/key.pem" -in "$tmp/cert.pem" -name "$name" \
    -passout "file:$kept/identity-password" -out "$kept/identity.p12" || { rm -rf "$tmp"; fail "openssl could not export the identity"; }
  chmod 600 "$kept/identity.p12"
  rm -rf "$tmp"
}

# codesign finds an identity by hash only in the user's keychain search list.
add_to_search_list() {
  local current k list=()
  current=$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//')
  grep -qxF "$kc" <<< "$current" && return 0
  while IFS= read -r k; do [ -n "$k" ] && list+=("$k"); done <<< "$current"
  security list-keychains -d user -s ${list[@]+"${list[@]}"} "$kc"
}

ensure() {
  umask 077
  mkdir -p "$kept"
  local hash
  hash=$(identity_hash)
  if [ -z "$hash" ]; then
    if [ ! -f "$kc" ]; then
      [ -f "$kc_pw" ] || new_password "$kc_pw"
      secure "create-keychain -p $(pw "$kc_pw") \"$kc\""
    fi
    [ -f "$kc_pw" ] || fail "$kc exists without its password file; move it aside and run again (kept/ restores the same identity)"
    if [ -f "$kept/identity.p12" ] && [ -f "$kept/identity-password" ]; then
      log "restoring $name from $kept"
    else
      log "creating $name"
      make_identity
    fi
    secure "unlock-keychain -p $(pw "$kc_pw") \"$kc\""
    secure "import \"$kept/identity.p12\" -k \"$kc\" -f pkcs12 -P $(pw "$kept/identity-password") -T /usr/bin/codesign"
    # Lets codesign use the key without a keychain dialog.
    secure "set-key-partition-list -S apple-tool:,apple:,codesign: -s -k $(pw "$kc_pw") \"$kc\""
    hash=$(identity_hash)
    [ -n "$hash" ] || fail "$kc holds no $name identity after the import"
  fi
  secure "unlock-keychain -p $(pw "$kc_pw") \"$kc\""
  # No auto-lock timeout, so a long build doesn't lose the key halfway.
  security set-keychain-settings "$kc"
  add_to_search_list
  log "$name ready: $hash"
  echo "$hash"
}

requirement() {
  local hash
  hash=$(identity_hash)
  [ -n "$hash" ] || fail "no $name identity yet; run: scripts/signing-identity.sh ensure"
  printf 'identifier "com.baney.silas" and certificate leaf = H"%s"\n' "$(tr 'A-F' 'a-f' <<< "$hash")"
}

case "${1:-}" in
  ensure) ensure ;;
  requirement) requirement ;;
  *) fail "usage: signing-identity.sh ensure | requirement" ;;
esac
