# Northbound website

Static HTML and CSS, using the wallet's logo and visual identity. No build step
or package installation is required. Open `website/index.html` to preview.

## Downloads and deployment

The download button uses GitHub's `/releases/latest/download/` URL. Each stable
release must attach its macOS Apple silicon installer using the same filename:
`Northbound-macos-arm64.dmg`. The release workflow must rename the built installer
to this name before uploading it. The link will not work until that asset exists
in a published stable release; prereleases do not advance the latest stable link.

No version or link changes are needed in the website for subsequent releases.
Add platform-specific buttons when their packages are available.

Deploy `index.html`, `styles.css`, and `assets/` from `website/` to
GitHub Pages using GitHub Actions. Assets use relative paths so both a repository
subpath and a custom domain work. Deployment workflows are configured separately.

## Design and review

This page extends the existing wallet identity. The surface direction lives in
`.impeccable/surfaces/website-index-html.md`; current values come from
`wallet/src/ui/ui.css`, where implementation has moved beyond some older
`DESIGN.md` tokens. This website does not redefine the global design system.

The site preserves the wallet's ink (`#202e40`), muted text (`#596778`), action
blue (`#255fb1`), hover blue (`#194d94`), ground (`#f6f7f9`), and hairline
(`#dce2e9`). The dark ownership section uses the logo's
`#243b59`. `assets/logo.svg` is copied unchanged from `wallet/assets/logo.svg`.
System typography, 6px button corners, and flat sections carry the same identity;
larger headings and spacing belong to this public product page.

The source includes keyboard focus, a skip link, reduced-motion handling, and
responsive layouts at 850px and 580px. These are implementation checks, not
rendered accessibility or visual approval. Desktop/mobile rendering, keyboard
navigation, zoom, and overflow still need browser verification before publishing.
