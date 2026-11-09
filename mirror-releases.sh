#!/usr/bin/env bash
# Mirror every non-draft release of the canonical repository into this one.
# Existing releases are skipped; their notes are updated when the source notes change.
set -euo pipefail
SOURCE="Vanir20342222/vidcue"
TARGET="Vanir20342222/ytui"

# Oldest first so the mirror builds up in release order.
tags=$(gh api "repos/$SOURCE/releases?per_page=100" --paginate --jq '.[] | select(.draft == false) | .tag_name' | tac)
latest=$(gh api "repos/$SOURCE/releases/latest" --jq .tag_name 2>/dev/null || echo "")

for tag in $tags; do
  info=$(gh api "repos/$SOURCE/releases/tags/$tag")
  body=$(printf '%s' "$info" | jq -r .body)
  name=$(printf '%s' "$info" | jq -r '.name // ""')
  prerelease=$(printf '%s' "$info" | jq -r .prerelease)
  tmp=$(mktemp -d)
  notes="$tmp/notes.md"
  printf '%s' "$body" > "$notes"

  if gh release view "$tag" --repo "$TARGET" >/dev/null 2>&1; then
    gh release view "$tag" --repo "$TARGET" --json body --jq .body > "$tmp/local.md"
    if ! cmp -s "$tmp/local.md" "$notes"; then
      gh release edit "$tag" --repo "$TARGET" --notes-file "$notes" --title "$name"
    fi
    rm -rf "$tmp"
    continue
  fi

  assets_dir="$tmp/assets"
  mkdir -p "$assets_dir"
  printf '%s' "$info" | jq -r '.assets[] | [.name, .browser_download_url] | @tsv' | while IFS=$'\t' read -r asset url; do
    curl -sSL --retry 3 -o "$assets_dir/$asset" "$url"
  done

  if [ "$prerelease" = "true" ] || [ "$tag" != "$latest" ]; then
    gh release create "$tag" --repo "$TARGET" --title "$name" --notes-file "$notes" --prerelease "$assets_dir"/*
  else
    gh release create "$tag" --repo "$TARGET" --title "$name" --notes-file "$notes" --latest=false "$assets_dir"/*
  fi
  rm -rf "$tmp"
done
