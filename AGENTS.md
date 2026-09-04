# Instructions for repository agents

These instructions apply to the whole repository and are vendor-neutral.

## Installing WiGigaDict for a user

Follow `docs/AGENT_INSTALL.md` exactly. Do not bypass checksum verification, SmartScreen, antivirus, or Windows policy. Do not request elevation for the per-user installer. Never download a speech model before the user has seen and accepted its size and license. Never delete the local archive or `%LOCALAPPDATA%\WiGigaDict` as part of an update.

## Working on the repository

- The implementation roadmap is `context/architecture/08-дорожная-карта/roadmap.md`. Start with the first open item relevant to the user's request; do not silently broaden scope.
- For implemented behavior, code is the source of truth. Keep architecture documents synchronized and record significant decisions in `context/architecture/06-решения/журнал-решений/`.
- Use `scripts/dev.ps1`, `scripts/build.ps1`, and `scripts/quality.ps1` instead of inventing parallel build paths.
- Preserve the local-first and no-silent-loss invariants. Recognition runtime code must not gain a network client.
- Never commit `.env` files, logs, model weights, private recordings, generated diagnostics, signing private keys, or files from user data directories.
- `apps/desktop/src-tauri/catalog.json` and `catalog.sig` are public signed release inputs. Their private signing key must stay outside the working tree.
- Before committing, run the checks appropriate to the change and inspect `git diff --check` plus the staged file list.
