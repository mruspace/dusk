#!/bin/sh
# Check that the README and the page quote what the code prints.
#
#   scripts/check-readme.sh         every string in docs/data.json's "display"
#                                   object must be in README.md, and the
#                                   page's static fallbacks must match it
#   scripts/check-readme.sh --run   also build dusk, run it, and diff the
#                                   README's output blocks against fresh output
#
# An output block is the fenced block right after a marker line:
#   <!-- output: default -->   all of `dusk`
#   <!-- output: ground -->    all of `dusk --ground`
#   <!-- output: sweep -->     all of `dusk --sweep`
#   <!-- excerpt: sweep -->    consecutive lines of `dusk --sweep`
#
# POSIX sh, sed, awk and grep only.

set -eu
cd "$(dirname "$0")/.."

run=0
case "${1:-}" in
  --run) run=1 ;;
  "") ;;
  *) echo "usage: $0 [--run]" >&2; exit 2 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
fail=0
json=docs/data.json
[ -f "$json" ] || { echo "no $json: run cargo run --release --bin charts" >&2; exit 1; }

# The generated block says where the data came from.
gen=$(sed -n 's/.*"generated":{\([^}]*\)}.*/\1/p' "$json")
echo "data.json generated: $gen"
case "$gen" in
  *-dirty*) echo "warning: data.json was generated from a tree with uncommitted changes" ;;
esac

# The display object is flat: "key":"value" pairs with no commas or quotes
# inside a value, so it splits on commas.
display=$(sed -n 's/.*"display":{\([^}]*\)}.*/\1/p' "$json")
[ -n "$display" ] || { echo "no display object in $json" >&2; exit 1; }
printf '%s\n' "$display" | tr ',' '\n' > "$tmp/display"

value() {
  sed -n "s/^\"$1\":\"\(.*\)\"\$/\1/p" "$tmp/display"
}

checked=0
while IFS= read -r pair; do
  key=$(printf '%s\n' "$pair" | sed 's/^"\([^"]*\)":.*$/\1/')
  val=$(value "$key")
  checked=$((checked + 1))
  if ! grep -qF -- "$val" README.md; then
    echo "README.md does not quote $key: $val"
    fail=1
  fi
done < "$tmp/display"
echo "README.md: checked $checked display strings"

# The page fills these from data.json, but shows its static text first and
# without JavaScript, so that text must match too.
for key in work_vs_tmr fewer_vs_simplex risk_shrink risk_tmr break_even voyager work_vs_unchecked \
  wrong_vs_unchecked work_vs_cold_01 work_vs_cold_10 risk_cold_01 risk_cold_10 \
  nodes2_work_vs_cold_01 nodes2_work_vs_cold_10; do
  val=$(value "$key")
  if [ -z "$val" ]; then
    echo "data.json has no display string $key"
    fail=1
  elif ! grep -qF -- ">$val" docs/index.html; then
    echo "docs/index.html static text does not match $key: $val"
    fail=1
  fi
done

if [ "$run" -eq 1 ]; then
  cargo build --release --quiet --bin dusk
  bin=target/release/dusk

  # The fenced block after the marker line, without its fences.
  block() {
    awk -v m="$1" '
      $0 == m { found = 1; next }
      found && !inside && /^```/ { inside = 1; next }
      inside && /^```/ { exit }
      inside { print }
    ' README.md
  }
  # Drop timing lines, the one thing allowed to differ between runs.
  untimed() {
    grep -v -E '^(took|elapsed|time) ' || true
  }

  for mode in default ground sweep; do
    case "$mode" in
      default) args="" ;;
      ground) args="--ground" ;;
      sweep) args="--sweep" ;;
    esac
    echo "running dusk $args"
    # shellcheck disable=SC2086
    "$bin" $args | untimed > "$tmp/$mode.out"

    block "<!-- output: $mode -->" | untimed > "$tmp/$mode.readme"
    if [ -s "$tmp/$mode.readme" ]; then
      if ! diff -u "$tmp/$mode.readme" "$tmp/$mode.out"; then
        echo "README.md output block '$mode' does not match dusk $args"
        fail=1
      fi
    fi

    # An excerpt must appear as consecutive lines of the output.
    block "<!-- excerpt: $mode -->" | untimed > "$tmp/$mode.excerpt"
    if [ -s "$tmp/$mode.excerpt" ]; then
      first=$(sed -n 1p "$tmp/$mode.excerpt")
      lines=$(wc -l < "$tmp/$mode.excerpt")
      at=$(grep -n -x -F -- "$first" "$tmp/$mode.out" | sed -n '1s/:.*//p')
      if [ -z "$at" ]; then
        echo "README.md excerpt '$mode' starts with a line dusk $args does not print:"
        echo "  $first"
        fail=1
      else
        sed -n "${at},$((at + lines - 1))p" "$tmp/$mode.out" > "$tmp/$mode.slice"
        if ! diff -u "$tmp/$mode.excerpt" "$tmp/$mode.slice"; then
          echo "README.md excerpt '$mode' does not match dusk $args"
          fail=1
        fi
      fi
    fi

    if [ ! -s "$tmp/$mode.readme" ] && [ ! -s "$tmp/$mode.excerpt" ]; then
      echo "README.md has no output block for '$mode'"
      fail=1
    fi
  done
fi

if [ "$fail" -ne 0 ]; then
  echo "check-readme: FAILED"
  exit 1
fi
echo "check-readme: ok"
