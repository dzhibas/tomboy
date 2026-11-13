# Contributing to tomboy

Thanks for helping. tomboy is a small personal launcher, so changes stay small and concrete.

## General

- Open an issue first for anything bigger than a fix, so we can agree on the shape.
- Branch from `main`; every change reaches `main` through a pull request.
- Build and test before you push:

  ```sh
  cargo build
  cargo test
  cargo run -- --snap shot.png --scene <scene>   # for UI changes; scenes are listed in README.md
  ```

- Match the code around you: short functions, comments that say why, no new dependencies
  without a reason.
- A new config option goes in `src/config.rs`, the README's settings table and
  `src/skill.md`, so the agent knows it too.
- Write docs in the README's style: plain, short sentences, tables where they help.

## Extensions

Extensions live in [`extensions/`](extensions), one folder each. How to write one:
[docs/extensions.md](docs/extensions.md).

### Pull request flow

1. Fork tomboy and clone your fork.
2. Inside `extensions/`, run `tomboy ext new <id>`. The id is lowercase letters, digits and
   dashes, and is the folder name.
3. Develop with `tomboy ext dev extensions/<id>` (from the repository root); Ctrl+R (⌘R)
   reloads. Fill in `tomboy.toml` (`author` is your GitHub login) and `README.md`.
4. Run `tomboy ext check extensions/<id>`.
5. Run `tomboy ext index` to regenerate `extensions/index.json`.
6. Commit the folder and `index.json` together.
7. Open a pull request and fill in the template.
8. After it is merged, add your line to [`.github/CODEOWNERS`](.github/CODEOWNERS)
   (`/extensions/<id>/ @<you> @dzhibas`) in a small PR of its own, or ask the maintainer
   to. It can't go in the extension PR: that PR may only touch `extensions/`.

Rules CI enforces:

- **One extension per pull request.** Changes to two extension folders fail.
- **Don't mix core changes in.** A PR that changes an extension folder and any file outside
  `extensions/` fails. Send tomboy changes, like a new API, in their own PR first.
- **Bump the version.** Any change to an existing extension must raise its `version`
  (`1.0.0` → `1.0.1`, `1.1.0`, …). Users only get updates that have a higher version.
- **Keep the index current.** `tomboy ext index --check` must pass.
- `tomboy ext check` must pass for the changed extension.

Changing someone else's extension is welcome; the author is asked to review it (see
[CODEOWNERS](#codeowners)).

### Review checklist (maintainers)

Every new extension and every change to one is read in full before merge. Check:

- [ ] **Permissions are minimal and justified.** Each host in `[permissions] net` is needed,
      and the PR says why.
- [ ] **Network hosts match the description.** A weather extension talks to a weather API,
      not to an analytics host or a server the author runs for no stated reason.
- [ ] **No obfuscated or minified code.** Every line is readable; no encoded blobs, no code
      built from strings.
- [ ] **Handles offline and errors.** Network calls are in `pcall` or checked; a failed
      request shows a message and leaves tomboy usable. Try it with the network off.
- [ ] **No secrets or personal data** in the folder; nothing sent to a host the user wouldn't
      expect.
- [ ] **README present**: what it does, keys and actions, preferences and their choices.
- [ ] **Licence is OK**: an open-source licence in `license`, and any copied code or data is
      compatible with it.
- [ ] **Version bumped** on changes to an existing extension (CI checks this too).
- [ ] **Manifest looks right**: `author` is the PR author (or an agreed maintainer), titles
      and keywords are sensible, `api` is the lowest that works.
- [ ] **CODEOWNERS line** for a new extension, added after merge (separate PR).
- [ ] **It works**: run `tomboy ext dev extensions/<id>` locally, or look at the screenshot.

When an update adds a permission, users are asked again before it installs; still look at
why.

### CODEOWNERS

[`.github/CODEOWNERS`](.github/CODEOWNERS) says who must review what:

- `/extensions/ @dzhibas`: every extension change needs a maintainer's approval.
- `/extensions/<id>/ @<author> @dzhibas`: one line per extension. GitHub requests a review
  from both when someone changes that folder, so authors hear about changes to their
  extension.
- `/.github/ @dzhibas`: CI, templates and CODEOWNERS itself, so nobody can change the rules
  or add themselves as an owner without the maintainer.

In CODEOWNERS the last matching line wins, so a folder's own line replaces the
`/extensions/` line for files in that folder. That is why `@dzhibas` is on every line.

With "Require review from Code Owners", an approval from any one owner of the matching line
satisfies it. So an author can approve a change to their own extension made by someone else.
The maintainer still has to approve the author's own PRs (GitHub never counts a PR author's
approval) and every PR that touches `extensions/index.json`, which only `/extensions/`
covers.

### Removing an extension

A maintainer may delete an extension folder (broken, abandoned, or unsafe). Users who have it
see "Removed from registry" in the store and can uninstall it; tomboy doesn't delete it for
them.

## Repository settings

These are set in GitHub, not in the repository, so they are written down here to re-apply.

Settings › Branches › branch protection rule for `main`:

| Setting | Value |
| --- | --- |
| Require a pull request before merging | on |
| Required approvals | 1 |
| Dismiss stale pull request approvals when new commits are pushed | on |
| Require review from Code Owners | on |
| Require status checks to pass before merging | on |
| Required status check | `extensions` (the job in `.github/workflows/extensions.yml`) |
| Require branches to be up to date before merging | on |
| Allow force pushes | off |
| Allow deletions | off |
| Do not allow bypassing the above settings (include administrators) | optional; off lets the maintainer merge core PRs alone |

The same with the GitHub CLI (needs admin rights on the repository):

```sh
gh api -X PUT repos/dzhibas/tomboy/branches/main/protection \
  -H "Accept: application/vnd.github+json" \
  --input - <<'EOF'
{
  "required_status_checks": {
    "strict": true,
    "contexts": ["extensions"]
  },
  "enforce_admins": false,
  "required_pull_request_reviews": {
    "required_approving_review_count": 1,
    "require_code_owner_reviews": true,
    "dismiss_stale_reviews": true
  },
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false
}
EOF
```

`strict: true` is "require branches to be up to date". Check the result with
`gh api repos/dzhibas/tomboy/branches/main/protection`.

The `extensions` workflow only runs when a PR touches `extensions/**`, `src/ext/**` or the
workflow. A required check that doesn't run blocks the merge, so a PR that touches none of
those shows `extensions` as expected but pending. Either have the maintainer merge such PRs
as an administrator (with `enforce_admins` off), or drop the `paths` filter from the
workflow's `pull_request` trigger so it runs on every PR.
