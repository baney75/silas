# shellcheck shell=bash disable=SC2154 # REPO, draft and tag come from the sourcing script
# Sourced by release-stable.sh. Every call names the draft by its release id ($draft) or one of its
# asset ids, never by tag: if a promotion re-run replaces the draft, these fail against the old id
# instead of writing into the new draft. Needs REPO and draft set.

draft_assets() { # prints "name<TAB>asset id" for each asset of the draft
  gh api "repos/$REPO/releases/$draft/assets" --paginate -q '.[] | [.name, (.id | tostring)] | @tsv'
}

draft_download() { # dir glob...: download the draft's assets whose names match any glob
  local dir=$1 list name id glob
  shift
  list=$(draft_assets) || return 1
  while IFS=$'\t' read -r name id; do
    [ -n "$name" ] || continue
    for glob in "$@"; do
      # shellcheck disable=SC2254 # the glob is meant to match
      case "$name" in $glob)
        gh api -H "Accept: application/octet-stream" "repos/$REPO/releases/assets/$id" > "$dir/$name" || return 1
        break ;;
      esac
    done
  done <<<"$list"
}

draft_upload() { # file: replace the draft's asset of the same name
  local f=$1 name list old
  name=$(basename "$f")
  list=$(draft_assets) || return 1
  old=$(awk -F'\t' -v n="$name" '$1 == n {print $2}' <<<"$list")
  if [ -n "$old" ]; then
    gh api -X DELETE "repos/$REPO/releases/assets/$old" >/dev/null || return 1
  fi
  gh api -X POST -H "Content-Type: application/octet-stream" --input "$f" \
    "https://uploads.github.com/repos/$REPO/releases/$draft/assets?name=$name" >/dev/null
}

draft_edit() { # title notes
  # Name the tag on every edit: GitHub turns a draft whose tag doesn't exist yet into
  # "untagged-..." when a PATCH leaves tag_name out (v0.1.7, 2026-11-22).
  gh api -X PATCH "repos/$REPO/releases/$draft" -f tag_name="$tag" -f name="$1" -f body="$2" >/dev/null
}
