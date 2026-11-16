# Northbound website

Static HTML and CSS with a small JavaScript screenshot carousel, using the wallet's logo and visual identity. Open
`website/index.html` to preview without installing anything. Release deployment
uses a small Python preparation step to fill in release metadata.

## Downloads and deployment

The independent **Deploy website** workflow publishes this directory to GitHub
Pages. It runs when website files change on `main`, when a release build finishes
successfully, or manually from **Actions → Deploy website → Run workflow**.
It never builds the wallet. The macOS, Windows, and Linux workflows build and upload packages.

Each deployment uses website source from `main` and the most recently published
release or prerelease that already has its macOS ZIP, Windows x64 MSI, Linux x64
AppImage, and all three checksums attached.
A newer release still being built does not hide the previous working download.
The page labels prereleases explicitly. Rerunning an older release cannot roll
the website back because selection always uses publication dates and available
assets. Manual wallet packaging runs do not trigger a website deployment.

If no published release has all three packages and checksums, the workflow
reports that downloads are not ready and skips upload and deployment successfully.
The existing website stays online. API failures and invalid release metadata
still fail the workflow.

`scripts/build-website.py` reads the published release metadata and fills in the
version and all three download URLs. It requires `Northbound-<tag>-macos-arm64.zip`,
`Northbound-<tag>-windows-x64.msi`, `Northbound-<tag>-linux-x64.AppImage`, and each package's `.sha256` file before
deployment, so a missing download fails the job instead of publishing a broken
link. Generated files are uploaded as a Pages artifact; source HTML is not
rewritten or committed.
Without this step, the preview's download links open the releases list.

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

Use a new output directory for each build. Linux downloads use the x64 AppImage;
Windows downloads use the x64 MSI installer.

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

The hero pairs the headline with its introduction and download action above the
wallet screenshot carousel. Downloads use a three-column platform row that stacks on phones.
Installation guidance covers the macOS ZIP, unsigned Windows MSI, and Linux
AppImage. A short FAQ explains recovery and the server trust boundary; the footer
links to GitHub Issues for help and bug reports.
Screenshots use `assets/wallet-<screen>.png` for the supplied originals and matching
`.avif` files for the website: `send`, `history`, `list`, and `welcome`.
The AVIF files preserve the original dimensions and transparency, converted with
ImageMagick at quality 75. To regenerate a screenshot, run:

```sh
magick website/assets/wallet-send.png -quality 75 website/assets/wallet-send.avif
```

The carousel uses a fixed 27:16 screenshot area without an outer frame, with
direct Welcome, Send, History, and Wallets selectors and a visible selected state.
It supports previous/next buttons, left/right arrow keys when its
image track is focused, and native touch scrolling. Slides advance every five
seconds, with a Play/Pause control. Rotation pauses on hover, keyboard focus,
touch interaction, and hidden browser tabs. Reduced-motion preferences disable
automatic rotation by default; visitors can still choose Play. Without JavaScript, screenshots remain
available in the horizontally scrollable track. Release generation copies
`carousel.js` alongside the HTML and CSS.
The styled carousel was checked at 1440px, 768px, 390px, and 320px with no
horizontal overflow. Browser checks covered selectors, previous/next wraparound,
keyboard navigation, image loading, reduced motion, and the no-JavaScript fallback.

The source includes keyboard focus, a skip link, reduced-motion handling, and
responsive layouts at 850px and 580px. Headless Chrome checks at 1440px, 768px,
390px, and 320px previously verified no horizontal overflow and screenshot loading.
The Windows download update was visually checked at 1440px and 390px with no
horizontal overflow. Release selection was checked with a newer incomplete
release and then with both packages present.
Manual keyboard, screen-reader, and zoom checks remain outstanding.
The installation guidance, FAQ, and footer were checked in headless Chrome at
1440px, 768px, 390px, and 320px with no horizontal overflow. Desktop and phone
screenshots were reviewed, and release generation passed with fixture metadata.
