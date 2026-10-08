#!/bin/sh
# Build the docs site in docs/ (served by GitHub Pages) from templates/,
# the man page and the license. Needs mandoc. Run it through `make docs`.
#
#   templates/page.html   the page skeleton: {{TITLE}}, {{CLASS}}, {{NAV}}, {{CONTENT}}
#   templates/nav.html    the nav bar shared by every page
#   templates/index.html  the home page content, written by hand
#
# docs/style.css and docs/desktop.png are edited directly, not generated.

set -eu

DOCS=docs
TEMPLATES=templates
MAN_PAGE=man/bsdt.1

command -v mandoc >/dev/null || {
    echo "mandoc not found; install it (e.g. apt install mandoc, brew install mandoc)" >&2
    exit 1
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# render TITLE CLASS CONTENT_FILE OUTPUT
render() {
    awk -v title="$1" -v class="$2" -v nav="$TEMPLATES/nav.html" -v content="$3" '
        /\{\{NAV\}\}/ { while ((getline line < nav) > 0) print line; close(nav); next }
        /\{\{CONTENT\}\}/ { while ((getline line < content) > 0) print line; close(content); next }
        { gsub(/\{\{TITLE\}\}/, title); gsub(/\{\{CLASS\}\}/, class); print }
    ' "$TEMPLATES/page.html" > "$4"
    echo "wrote $4"
}

mkdir -p "$DOCS"

render "bsdt: repeatable FreeBSD development VMs" home "$TEMPLATES/index.html" "$DOCS/index.html"

# The man page body, from its header table up to (not including) the footer
# table, with cross references linked to FreeBSD's online manual pages.
mandoc -T lint -W warning "$MAN_PAGE"
mandoc -T html -O 'man=https://man.freebsd.org/cgi/man.cgi?query=%N&sektion=%S' "$MAN_PAGE" |
    awk '/<table class="head">/ { keep = 1 } /<table class="foot">/ { keep = 0 } keep' > "$tmp/man.html"
render "bsdt(1) manual page" manual "$tmp/man.html" "$DOCS/bsdt.1.html"

# The license is plain text: escape it and keep its layout.
{
    echo '<pre class="license">'
    sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' LICENSE
    echo '</pre>'
} > "$tmp/license.html"
render "bsdt license" license "$tmp/license.html" "$DOCS/LICENSE.html"
