# Project tasks. Needs only cargo; update-screenshot also uses awk.

default:
    @just --list

# Render the README screenshot from the real UI code and print it
screenshot:
    @cargo test --quiet readme_screenshot -- --ignored --nocapture | sed -n '/^┌/,/^ ttyUSB0/p'

# Replace the screenshot block in README.md with a fresh render
update-screenshot:
    #!/usr/bin/env bash
    set -euo pipefail
    SHOT="$(just screenshot)" awk '
        /^```text$/ && !done { print; print ENVIRON["SHOT"]; skip = 1; next }
        skip && /^```$/ { skip = 0; done = 1 }
        !skip { print }
    ' README.md > README.md.tmp
    mv README.md.tmp README.md
    git --no-pager diff --stat -- README.md
