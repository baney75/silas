#!/usr/bin/env bash
# Exercises scripts/signing-identity.sh with stand-in security, openssl and uname, so it runs
# on any CI host. Checks that no password reaches a command's arguments and that a lost
# keychain comes back with the same identity from kept/.
#   bash scripts/signing-identity-test.sh
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/log"
export LOG="$tmp/log" SILAS_SIGNING_DIR="$tmp/Signing Dir" OPENSSL="$tmp/bin/openssl"
hash=ABCDEF0123456789ABCDEF0123456789ABCDEF01

cat > "$tmp/bin/uname" <<'EOF'
#!/usr/bin/env bash
echo Darwin
EOF
cat > "$tmp/bin/openssl" <<'EOF'
#!/usr/bin/env bash
echo "openssl $*" >> "$LOG/argv"
case "$1" in
  rand) n=$(( $(cat "$LOG/n" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$LOG/n"; echo "secret$n-5f3a9c" ;;
  version) echo "LibreSSL 3.3.6" ;;
  req|pkcs12) prev=; for a in "$@"; do case "$prev" in -out|-keyout) echo fake > "$a" ;; esac; prev=$a; done ;;
esac
EOF
cat > "$tmp/bin/security" <<EOF
#!/usr/bin/env bash
echo "security \$*" >> "\$LOG/argv"
case "\$1" in
  -i)
    while IFS= read -r line; do
      echo "\$line" >> "\$LOG/stdin"
      case "\$line" in
        create-keychain*) touch "\$(sed -n 's/.*"\(.*\)"\$/\1/p' <<< "\$line")" ;;
        import*) touch "\$LOG/imported" ;;
      esac
    done ;;
  find-identity)
    [ -f "\$LOG/imported" ] && echo '  1) $hash "Silas Local Signing" (CSSMERR_TP_NOT_TRUSTED)'
    echo "     1 identities found" ;;
  list-keychains)
    if [ "\${4:-}" = -s ]; then shift 4; printf '    "%s"\n' "\$@" > "\$LOG/search"; else cat "\$LOG/search" 2>/dev/null || echo '    "/Users/me/Library/Keychains/login.keychain-db"'; fi ;;
esac
EOF
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH"

failures=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; failures=$((failures + 1)); }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }
run() { bash scripts/signing-identity.sh "$@" 2>>"$LOG/stderr"; }

out=$(run ensure) || bad "ensure exits 0"
check "ensure prints the identity's SHA-1" '[ "$out" = "$hash" ]'
check "a keychain is created" 'grep -q "^create-keychain " "$LOG/stdin"'
check "the identity is imported from kept/" 'grep -q "^import \"$SILAS_SIGNING_DIR/kept/identity.p12\"" "$LOG/stdin"'
check "codesign may use the key without a dialog" 'grep -q "^set-key-partition-list " "$LOG/stdin"'
check "the keychain joins the search list" 'grep -qF "$SILAS_SIGNING_DIR/silas-signing.keychain-db" "$LOG/search"'
check "the login keychain stays in the search list" 'grep -q login.keychain-db "$LOG/search"'
for f in keychain-password kept/identity-password kept/identity.p12; do
  check "$f is private" '[ "$(stat -c %a "$SILAS_SIGNING_DIR/$f" 2>/dev/null || stat -f %Lp "$SILAS_SIGNING_DIR/$f")" = 600 ]'
done
for f in keychain-password kept/identity-password; do
  secret=$(cat "$SILAS_SIGNING_DIR/$f")
  check "the $f value never appears in argv" '! grep -qF "$secret" "$LOG/argv"'
  check "the $f value travels on stdin" 'grep -qF "$secret" "$LOG/stdin"'
done

out=$(run ensure) || bad "a second ensure exits 0"
check "a second ensure keeps the identity" '[ "$out" = "$hash" ] && [ "$(grep -c "^openssl req" "$LOG/argv")" = 1 ]'

# A lost keychain: kept/ must bring back the same identity, not mint a new one.
rm "$SILAS_SIGNING_DIR/silas-signing.keychain-db" "$LOG/imported"
out=$(run ensure) || bad "ensure after a lost keychain exits 0"
check "a lost keychain restores the same identity" '[ "$out" = "$hash" ]'
check "a restore makes no new certificate" '[ "$(grep -c "^openssl req" "$LOG/argv")" = 1 ]'
check "a restore keeps the .p12 password" '[ "$(grep -c "^import " "$LOG/stdin")" = 2 ]'

check "the requirement names the bundle and the leaf" \
  '[ "$(run requirement)" = "identifier \"com.baney.silas\" and certificate leaf = H\"$(tr A-F a-f <<< $hash)\"" ]'

[ "$failures" = 0 ] && echo "signing-identity: all passed" || { echo "signing-identity: $failures failed"; exit 1; }
