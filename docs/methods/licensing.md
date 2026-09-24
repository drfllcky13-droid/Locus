# Licensing, crash reports and releases

## Licence tiers

Lotus has three tiers, as feature flags on one program (SPEC §1):

| Tier | Includes |
|---|---|
| Diagram | 2D diagramming, hand measurements, the symbol library, reports; photos as diagram underlays |
| Analyst | Everything in Diagram, plus 3D scenes, point clouds and scan import, every analysis tool, photogrammetry, animation and the case package |
| Analyst Plus | Everything in Analyst, plus scan registration and volumetric crush comparison |

**The licence file** is a small JSON file (`.locus-license`) holding the licensee, tier, licence id, issue date and optional expiry, with an Ed25519 signature over those fields.
- Lotus holds only the publisher's public key, so a changed or forged file doesn't verify: raising the tier breaks the signature.
- No network is involved: it works offline and on air-gapped machines.
- Install one under Help → About → Install a licence…. It is kept in the app's configuration folder.
- An expired licence, or one that doesn't verify, is reported with the reason.

**Enforcement.**
- The commands that belong to a tier refuse outside it, with a message naming the licence needed: scan import, the analyses, photogrammetry, 3D scenes, animation and render, the case package, registration and volumetric crush.
- The UI hides what the licence doesn't include.
- Opening, viewing and printing records that already exist is never blocked, so a case made under a higher licence stays readable.

**No licence.** Without a licence file, or with one that doesn't verify, Lotus runs as an **unlicensed evaluation** with every tool, and says so in the side panel and in About. Whether evaluations stay unrestricted is the publisher's decision before a commercial release (docs/ADDISON-TODO.md).

**Making licences.** `locus-license keygen --out KEY.txt` makes the publisher's key pair once; the public half goes in `src-tauri/src/license_cmds.rs`. `locus-license sign --key KEY.txt --id … --licensee … --tier analyst [--expires YYYY-MM-DD] --out FILE.locus-license` signs a licence. The private key must never enter the repository or the app.

## Crash reports

- **When Lotus fails**, it writes a crash report to its own log folder: an unexpected error in the program (a panic, on any thread) or in its view (an uncaught exception).
- **What a report holds:** the version, the system, when it happened, the message and where in Lotus's code it happened.
- **What it never holds:** anything that looks like a file path is removed from the message and the backtrace, so a case folder's or an evidence file's name can't appear. There is no project data.
- **Nothing is sent.** On the next start Lotus says a report was saved and offers to show it. The user can copy it to send to the developer themselves, or delete it.

This is the spec's "crash reporting that never uploads case data", taken to the point of never uploading at all without the user. Forensic workstations are often offline, and anything leaving the machine has to be justified.

## Installer and updates

- **The installer:** a Windows MSI from Tauri's bundler (WiX), signed with the publisher's code-signing certificate. Tauri's bundler downloads the WiX toolset on first use, which needs approval. The certificate, the approval and the publisher domain (for the app's identifier) are on docs/ADDISON-TODO.md. Until then, releases are unsigned builds.
- **Updates:** Tauri's updater, from signed update files published with each release. An update is only installed if its signature matches the key built into the app. Updates contain only the program: no case data goes anywhere, and nothing is sent but the version check.
- **Not yet switched on.** The updater needs the update-signing key (a CI secret) and the place releases are published, which are the publisher's to set up.
