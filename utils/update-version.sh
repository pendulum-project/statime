#!/usr/bin/env bash

if [ "$#" -lt 1 ]; then
    echo "Missing new version specifier"
    exit 1
fi

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd)
PROJECT_DIR=$(dirname "$SCRIPT_DIR")
NEW_VERSION="$1"
NOTICE_LINE="# NOTE: keep this part at the bottom of the file, do not change this line"

echo "Updating version in Cargo.toml"
sed -i 's/^version\s*=\s*".*"/version = "'"$NEW_VERSION"'"/' "$PROJECT_DIR/Cargo.toml"

echo "Updating workspace crate versions in Cargo.toml"
tmp_file="$PROJECT_DIR/Cargo.toml.tmp"
replace_flag=0
while IFS= read -r line; do
    if [ $replace_flag -eq 1 ]; then
        line=$(echo "$line" | sed 's/version\s*=\s*"[^"]*"/version = "'"$NEW_VERSION"'"/g')
    fi

    if [ "$line" = "$NOTICE_LINE" ]; then
        replace_flag=1
    fi

    echo "$line" >> "$tmp_file"
done < "$PROJECT_DIR/Cargo.toml"
mv "$tmp_file" "$PROJECT_DIR/Cargo.toml"

echo "Updating version in man pages"
sed -i 's/^title: STATIME(8) statime .*/title: STATIME(8) statime '"$NEW_VERSION"' | statime/' "$PROJECT_DIR"/docs/man/statime.8.md
sed -i 's/^title: STATIME.TOML(5) statime .*/title: STATIME.TOML(5) statime '"$NEW_VERSION"' | statime/' "$PROJECT_DIR"/docs/man/statime.toml.5.md

echo "Rebuilding precompiled man pages"
utils/generate-man.sh

echo "Rebuilding project"
(cd $PROJECT_DIR && cargo clean && cargo build --release)

echo "!!! Version changes complete, make sure that the changelog is synchronized"
