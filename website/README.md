# Northbound website

Static HTML and CSS, using the wallet's logo and visual identity. Open
`website/index.html` to preview without installing anything. Release deployment
uses a small Python preparation step to fill in release metadata.

## Downloads and deployment

The independent **Deploy website** workflow publishes this directory to GitHub
Pages. It runs when website files change on `main`, when a release build finishes
successfully, or manually from **Actions → Deploy website → Run workflow**.
It never builds the wallet. The macOS workflow only builds and uploads packages.

Each deployment uses website source from `main` and the most recently published
release or prerelease that already has its macOS ZIP and checksum attached.
A newer release still being built does not hide the previous working download.
The page labels prereleases explicitly. Rerunning an older release cannot roll
the website back because selection always uses publication dates and available
assets. Manual wallet packaging runs do not trigger a website deployment.

`scripts/build-website.py` reads the published release metadata and fills in the
version, macOS ZIP download URL, and release-notes URL. It requires both
`Northbound-<tag>-macos-arm64.zip` and its `.sha256` file before deployment, so a
missing download fails the job instead of publishing a broken link. Generated
files are uploaded as a Pages artifact; source HTML is not rewritten or committed.
Without this step, the preview's download link opens the releases list.

One-time repository setup:

1. Open **Settings → Pages → Build and deployment** and select **GitHub Actions**
   as the source.
2. If the `github-pages` environment restricts deployment branches, allow `main`;
   website deployment runs from the default branch.
3. Run **Deploy website** once there is a published release or prerelease with
   its download assets. This can reuse an existing release without a rebuild.

The website's custom domain is `northboundwallet.org`, configured in repository
Pages settings. In Cloudflare, use DNS-only `A` records at `@` pointing to
`185.199.108.153`, `185.199.109.153`, `185.199.110.153`, and `185.199.111.153`.
The `api` record continues to point to the wallet API server at `13.55.12.147`.
Enable **Enforce HTTPS** in Pages settings once its certificate is available.
Actions-based Pages deployment does not require a `CNAME` file. Assets use
relative paths, so the original `https://nova9.github.io/northbound/` subpath
also works when a custom domain is not configured.

To check a generated site locally:

```sh
gh api repos/nova9/northbound/releases/tags/<tag> > /tmp/northbound-release.json
python3 scripts/build-website.py /tmp/northbound-release.json /tmp/northbound-website
python3 -m http.server 8000 --directory /tmp/northbound-website
```

Use a new output directory for each build. Add platform-specific buttons when
their packages are available.

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
