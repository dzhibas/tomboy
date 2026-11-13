<!-- Adding or changing an extension? Fill in the first part and delete the second.
     A change to tomboy itself? Delete the first part. -->

## Extension

**Extension:** `extensions/<id>/` (new / update from x.y.z to x.y.z)

**What it does:**

**Screenshot:**
<!-- A screenshot of it running in tomboy (`tomboy ext dev extensions/<id>`), or
     `cargo run -- --snap shot.png --scene <scene>` if you added a scene. -->

**Permissions, and why each is needed:**
<!-- One line per host in [permissions] net, e.g.
     - hn.algolia.com: the Hacker News search API the list comes from -->

- [ ] One extension in this PR, and no files outside `extensions/`
- [ ] `tomboy ext check extensions/<id>` passes
- [ ] `tomboy ext index` run, and `extensions/index.json` committed
- [ ] `version` in `tomboy.toml` bumped (for a change to an existing extension)
- [ ] `README.md` says what it does, its keys and actions, and its preferences
- [ ] It handles being offline or an API error (tried with the network off)
- [ ] `author` in `tomboy.toml` is my GitHub login, and I will maintain this extension

## Other changes

**What and why:**

- [ ] `cargo build` and `cargo test` pass
- [ ] UI change: screenshot attached (`cargo run -- --snap shot.png --scene <scene>`)
- [ ] New config option documented in `README.md` and `src/skill.md`
